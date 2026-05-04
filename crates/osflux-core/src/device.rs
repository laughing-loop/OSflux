use std::collections::{HashMap, HashSet};
use std::process::Command;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{GET_LENGTH_INFORMATION, IOCTL_DISK_GET_LENGTH_INFO};
use windows::Win32::System::IO::DeviceIoControl;

const DEFAULT_SCAN_LIMIT: u32 = 32;

#[derive(Debug, Clone)]
pub struct PhysicalDevice {
    pub index: u32,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub model: Option<String>,
    pub interface_type: Option<String>,
    pub media_type: Option<String>,
    pub pnp_device_id: Option<String>,
    pub contains_system_volume: bool,
    pub access: DeviceAccess,
    pub note: Option<String>,
}

impl PhysicalDevice {
    pub fn is_likely_removable_target(&self) -> bool {
        let interface_is_usb = self
            .interface_type
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("USB"));

        let media_looks_removable = self.media_type.as_deref().is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("removable") || value.contains("external")
        });

        interface_is_usb || media_looks_removable
    }

    pub fn safety_label(&self) -> &'static str {
        if self.contains_system_volume {
            "system-drive"
        } else if self.is_likely_removable_target() {
            "candidate"
        } else {
            "internal-or-unknown"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAccess {
    Readable,
    MetadataOnly,
}

impl DeviceAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Readable => "readable",
            Self::MetadataOnly => "metadata-only",
        }
    }
}

pub struct Device {
    handle: HANDLE,
}

impl Device {
    pub fn open_raw(path: &str) -> Result<Self> {
        let handle = open_handle(
            path,
            FILE_GENERIC_WRITE.0,
            FILE_FLAG_OVERLAPPED | FILE_ATTRIBUTE_NORMAL,
        )?;

        Ok(Self { handle })
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }
}

pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    let metadata = query_disk_metadata();
    let mut devices = scan_physical_devices(DEFAULT_SCAN_LIMIT);
    let mut seen_indexes = HashSet::new();

    for device in &mut devices {
        seen_indexes.insert(device.index);
        if let Some(metadata) = metadata.get(&device.index) {
            device.apply_metadata(metadata);
        }
    }

    for metadata in metadata.values() {
        if seen_indexes.contains(&metadata.index) {
            continue;
        }

        devices.push(PhysicalDevice::from_metadata(metadata));
    }

    devices.sort_by_key(|device| device.index);
    devices
}

pub fn scan_physical_devices(limit: u32) -> Vec<PhysicalDevice> {
    (0..limit).filter_map(probe_physical_device).collect()
}

fn probe_physical_device(index: u32) -> Option<PhysicalDevice> {
    let path = format!(r"\\.\PhysicalDrive{index}");

    match open_handle(&path, FILE_GENERIC_READ.0, FILE_ATTRIBUTE_NORMAL) {
        Ok(handle) => {
            let size_bytes = query_device_length(handle);
            close_handle(handle);

            Some(PhysicalDevice {
                index,
                path,
                size_bytes,
                model: None,
                interface_type: None,
                media_type: None,
                pnp_device_id: None,
                contains_system_volume: false,
                access: DeviceAccess::Readable,
                note: None,
            })
        }
        Err(read_error) => match open_handle(&path, 0, FILE_ATTRIBUTE_NORMAL) {
            Ok(handle) => {
                close_handle(handle);

                Some(PhysicalDevice {
                    index,
                    path,
                    size_bytes: None,
                    model: None,
                    interface_type: None,
                    media_type: None,
                    pnp_device_id: None,
                    contains_system_volume: false,
                    access: DeviceAccess::MetadataOnly,
                    note: Some(format!("read access unavailable: {read_error}")),
                })
            }
            Err(_) => None,
        },
    }
}

impl PhysicalDevice {
    fn apply_metadata(&mut self, metadata: &DiskMetadata) {
        self.size_bytes = self.size_bytes.or(metadata.size_bytes);
        self.model = metadata.model.clone();
        self.interface_type = metadata.interface_type.clone();
        self.media_type = metadata.media_type.clone();
        self.pnp_device_id = metadata.pnp_device_id.clone();
        self.contains_system_volume = metadata.contains_system_volume;
    }

    fn from_metadata(metadata: &DiskMetadata) -> Self {
        Self {
            index: metadata.index,
            path: format!(r"\\.\PhysicalDrive{}", metadata.index),
            size_bytes: metadata.size_bytes,
            model: metadata.model.clone(),
            interface_type: metadata.interface_type.clone(),
            media_type: metadata.media_type.clone(),
            pnp_device_id: metadata.pnp_device_id.clone(),
            contains_system_volume: metadata.contains_system_volume,
            access: DeviceAccess::MetadataOnly,
            note: Some("listed by Windows disk metadata; direct handle unavailable".to_string()),
        }
    }
}

#[derive(Debug, Clone)]
struct DiskMetadata {
    index: u32,
    model: Option<String>,
    interface_type: Option<String>,
    media_type: Option<String>,
    size_bytes: Option<u64>,
    pnp_device_id: Option<String>,
    contains_system_volume: bool,
}

fn query_disk_metadata() -> HashMap<u32, DiskMetadata> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            DISK_METADATA_SCRIPT,
        ])
        .output();

    let Ok(output) = output else {
        return HashMap::new();
    };

    if !output.status.success() {
        return HashMap::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(parse_disk_metadata_line)
        .map(|metadata| (metadata.index, metadata))
        .collect()
}

fn parse_disk_metadata_line(line: &str) -> Option<DiskMetadata> {
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() < 7 {
        return None;
    }

    let index = fields[0].trim().parse::<u32>().ok()?;

    Some(DiskMetadata {
        index,
        model: optional_field(fields[1]),
        interface_type: optional_field(fields[2]),
        media_type: optional_field(fields[3]),
        size_bytes: fields[4].trim().parse::<u64>().ok(),
        pnp_device_id: optional_field(fields[5]),
        contains_system_volume: fields[6].trim().eq_ignore_ascii_case("true"),
    })
}

fn optional_field(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn open_handle(
    path: &str,
    desired_access: u32,
    flags: FILE_FLAGS_AND_ATTRIBUTES,
) -> Result<HANDLE> {
    let path_utf16: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        CreateFileW(
            PCWSTR(path_utf16.as_ptr()),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
}

fn query_device_length(handle: HANDLE) -> Option<u64> {
    let mut length = GET_LENGTH_INFORMATION::default();
    let mut bytes_returned = 0u32;

    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_DISK_GET_LENGTH_INFO,
            None,
            0,
            Some((&mut length as *mut GET_LENGTH_INFORMATION).cast()),
            std::mem::size_of::<GET_LENGTH_INFORMATION>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    result.ok()?;
    u64::try_from(length.Length).ok()
}

fn close_handle(handle: HANDLE) {
    if !handle.is_invalid() {
        unsafe {
            let _ = CloseHandle(handle);
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        close_handle(self.handle);
    }
}

const DISK_METADATA_SCRIPT: &str = r#"
$systemDrive = $env:SystemDrive

function Clean($value) {
    if ($null -eq $value) {
        return ""
    }

    return ([string]$value) -replace "`t", " " -replace "`r|`n", " "
}

Get-CimInstance Win32_DiskDrive | Sort-Object Index | ForEach-Object {
    $disk = $_
    $containsSystem = $false

    try {
        $partitions = Get-CimAssociatedInstance -InputObject $disk -Association Win32_DiskDriveToDiskPartition
        foreach ($partition in $partitions) {
            $logicalDisks = Get-CimAssociatedInstance -InputObject $partition -Association Win32_LogicalDiskToPartition
            foreach ($logicalDisk in $logicalDisks) {
                if ($logicalDisk.DeviceID -eq $systemDrive) {
                    $containsSystem = $true
                }
            }
        }
    } catch {}

    [string]::Join("`t", @(
        (Clean $disk.Index),
        (Clean $disk.Model),
        (Clean $disk.InterfaceType),
        (Clean $disk.MediaType),
        (Clean $disk.Size),
        (Clean $disk.PNPDeviceID),
        (Clean $containsSystem)
    ))
}
"#;
