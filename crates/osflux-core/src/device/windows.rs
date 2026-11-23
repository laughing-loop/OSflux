use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, FlushFileBuffers,
    FILE_ATTRIBUTE_NORMAL, FILE_DEVICE_DISK, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
    FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    DISK_GEOMETRY, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, GET_LENGTH_INFORMATION,
    IOCTL_DISK_GET_DRIVE_GEOMETRY, IOCTL_DISK_GET_LENGTH_INFO, IOCTL_DISK_UPDATE_PROPERTIES,
    IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER,
};
use windows::Win32::System::IO::DeviceIoControl;

use super::{
    is_placeholder_serial, BlockDevice, DeviceAccess, DeviceError, DeviceRead, DeviceResult,
    PhysicalDevice,
};
use crate::windows_io::{control_overlapped, query_overlapped, read_overlapped, write_overlapped};

const DEFAULT_SCAN_LIMIT: u32 = 32;
const VOLUME_LOCK_ATTEMPTS: u32 = 20;
const VOLUME_LOCK_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_VOLUME_NAME_CHARS: usize = 260;
const MAX_VOLUME_EXTENTS: usize = 32;
const DISK_EXTENTS_HEADER_SIZE: usize = 8;
const DISK_EXTENT_SIZE: usize = 24;

pub struct WindowsBlockDevice {
    handle: HANDLE,
    // Windows refuses raw writes to sectors of a mounted volume, so every
    // volume on the disk stays locked and dismounted while it is open.
    // Closing these handles releases the locks.
    locked_volumes: Vec<LockedVolume>,
}

impl WindowsBlockDevice {
    pub fn open_raw(path: &str) -> DeviceResult<Self> {
        Self::open(path)
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }
}

impl BlockDevice for WindowsBlockDevice {
    fn open(path: &str) -> DeviceResult<Self> {
        let handle = open_handle(
            path,
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_FLAG_OVERLAPPED | FILE_ATTRIBUTE_NORMAL,
        )
        .map_err(|error| DeviceError::Open {
            path: path.to_string(),
            message: error.to_string(),
        })?;
        // Built before the remaining steps so the handle is closed if one fails.
        let mut device = Self {
            handle,
            locked_volumes: Vec::new(),
        };

        let device_number: STORAGE_DEVICE_NUMBER =
            query_overlapped(handle, IOCTL_STORAGE_GET_DEVICE_NUMBER).map_err(|error| {
                DeviceError::Open {
                    path: path.to_string(),
                    message: format!("could not read the disk number: {error}"),
                }
            })?;

        // Only whole physical disks are opened for writing. A single volume or
        // a file on a disk can answer the same query with a partition number.
        if !is_whole_disk(&device_number) {
            return Err(DeviceError::Open {
                path: path.to_string(),
                message: "not a whole physical disk".to_string(),
            });
        }

        device.locked_volumes = lock_disk_volumes(device_number.DeviceNumber)?;

        Ok(device)
    }

    fn write_at(&self, offset: u64, buffer: &[u8]) -> DeviceResult<usize> {
        let written =
            write_overlapped(self.handle, buffer, offset).map_err(|error| DeviceError::Io {
                operation: "write_at",
                message: error.to_string(),
            })?;

        if written != buffer.len() {
            return Err(DeviceError::Io {
                operation: "write_at",
                message: format!(
                    "wrote {written} of {} bytes at offset {offset}",
                    buffer.len()
                ),
            });
        }

        Ok(written)
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<usize> {
        let read =
            read_overlapped(self.handle, buffer, offset).map_err(|error| DeviceError::Io {
                operation: "read_at",
                message: error.to_string(),
            })?;

        if read != buffer.len() {
            return Err(DeviceError::Io {
                operation: "read_at",
                message: format!("read {read} of {} bytes at offset {offset}", buffer.len()),
            });
        }

        Ok(read)
    }

    fn flush(&self) -> DeviceResult<()> {
        unsafe { FlushFileBuffers(self.handle) }.map_err(|error| DeviceError::Io {
            operation: "flush",
            message: error.to_string(),
        })
    }

    fn size(&self) -> DeviceResult<u64> {
        let length: GET_LENGTH_INFORMATION =
            query_overlapped(self.handle, IOCTL_DISK_GET_LENGTH_INFO).map_err(|error| {
                DeviceError::Io {
                    operation: "size",
                    message: error.to_string(),
                }
            })?;

        u64::try_from(length.Length).map_err(|_| DeviceError::Io {
            operation: "size",
            message: "device length unavailable".to_string(),
        })
    }

    fn logical_sector_size(&self) -> DeviceResult<u32> {
        query_sector_size(self.handle)
    }
}

impl Drop for WindowsBlockDevice {
    fn drop(&mut self) {
        // Best effort: have Windows re-read the partition table so the new
        // partitions appear once the volume locks are released.
        let _ = control_overlapped(self.handle, IOCTL_DISK_UPDATE_PROPERTIES);
        close_handle(self.handle);
        // `locked_volumes` drops after this, so the locks outlive the disk handle.
    }
}

/// A whole drive opened for reading only. Unlike [`WindowsBlockDevice`] it
/// leaves the drive's volumes mounted and usable.
pub struct WindowsReadOnlyDevice {
    handle: HANDLE,
    size_bytes: u64,
    sector_size_bytes: u32,
}

impl WindowsReadOnlyDevice {
    pub fn open(path: &str) -> DeviceResult<Self> {
        let open_error = |message: String| DeviceError::Open {
            path: path.to_string(),
            message,
        };
        let handle = open_handle(
            path,
            FILE_GENERIC_READ.0,
            FILE_FLAG_OVERLAPPED | FILE_ATTRIBUTE_NORMAL,
        )
        .map_err(|error| open_error(error.to_string()))?;
        // Built first so the handle is closed if a later step fails.
        let mut device = Self {
            handle,
            size_bytes: 0,
            sector_size_bytes: 512,
        };

        let device_number: STORAGE_DEVICE_NUMBER =
            query_overlapped(handle, IOCTL_STORAGE_GET_DEVICE_NUMBER)
                .map_err(|error| open_error(format!("could not read the disk number: {error}")))?;
        if !is_whole_disk(&device_number) {
            return Err(open_error("not a whole physical disk".to_string()));
        }

        let length: GET_LENGTH_INFORMATION =
            query_overlapped(handle, IOCTL_DISK_GET_LENGTH_INFO)
                .map_err(|error| open_error(format!("could not read the drive size: {error}")))?;
        device.size_bytes = u64::try_from(length.Length)
            .map_err(|_| open_error("the drive reported a negative size".to_string()))?;
        device.sector_size_bytes = query_sector_size(handle)?;

        Ok(device)
    }
}

impl DeviceRead for WindowsReadOnlyDevice {
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<()> {
        let read =
            read_overlapped(self.handle, buffer, offset).map_err(|error| DeviceError::Io {
                operation: "read_at",
                message: error.to_string(),
            })?;
        if read != buffer.len() {
            return Err(DeviceError::Io {
                operation: "read_at",
                message: format!("read {read} of {} bytes at offset {offset}", buffer.len()),
            });
        }
        Ok(())
    }

    fn size_bytes(&self) -> DeviceResult<u64> {
        Ok(self.size_bytes)
    }

    fn sector_size_bytes(&self) -> DeviceResult<u32> {
        Ok(self.sector_size_bytes)
    }
}

impl Drop for WindowsReadOnlyDevice {
    fn drop(&mut self) {
        close_handle(self.handle);
    }
}

fn query_sector_size(handle: HANDLE) -> DeviceResult<u32> {
    let geometry: DISK_GEOMETRY =
        query_overlapped(handle, IOCTL_DISK_GET_DRIVE_GEOMETRY).map_err(|error| {
            DeviceError::Io {
                operation: "read the sector size",
                message: error.to_string(),
            }
        })?;
    match geometry.BytesPerSector {
        512 | 1024 | 2048 | 4096 => Ok(geometry.BytesPerSector),
        other => Err(DeviceError::Io {
            operation: "read the sector size",
            message: format!("unexpected sector size {other}"),
        }),
    }
}

struct LockedVolume {
    handle: HANDLE,
}

impl Drop for LockedVolume {
    fn drop(&mut self) {
        close_handle(self.handle);
    }
}

struct VolumeSearch(HANDLE);

impl Drop for VolumeSearch {
    fn drop(&mut self) {
        unsafe {
            let _ = FindVolumeClose(self.0);
        }
    }
}

fn is_whole_disk(device_number: &STORAGE_DEVICE_NUMBER) -> bool {
    device_number.DeviceType == FILE_DEVICE_DISK.0 && device_number.PartitionNumber == 0
}

fn lock_disk_volumes(disk_number: u32) -> DeviceResult<Vec<LockedVolume>> {
    // On failure the volumes locked so far drop here and are released.
    volumes_on_disk(disk_number)?
        .iter()
        .map(|volume| lock_and_dismount_volume(volume))
        .collect()
}

fn volumes_on_disk(disk_number: u32) -> DeviceResult<Vec<String>> {
    let mut name = [0u16; MAX_VOLUME_NAME_CHARS];
    let search = unsafe { FindFirstVolumeW(&mut name) }.map_err(|error| DeviceError::Io {
        operation: "list volumes",
        message: error.to_string(),
    })?;
    let search = VolumeSearch(search);
    let mut volumes = Vec::new();

    loop {
        let volume = wide_to_string(&name);
        if volume_disk_numbers(&volume).contains(&disk_number) {
            volumes.push(volume);
        }

        if let Err(error) = unsafe { FindNextVolumeW(search.0, &mut name) } {
            if error.code() == ERROR_NO_MORE_FILES.to_hresult() {
                return Ok(volumes);
            }

            return Err(DeviceError::Io {
                operation: "list volumes",
                message: error.to_string(),
            });
        }
    }
}

/// Disk numbers a volume sits on. Volumes that cannot be opened or queried,
/// such as an empty card reader, are skipped: if one did sit on the target,
/// Windows would refuse writes to its sectors, so skipping it fails safe.
fn volume_disk_numbers(volume_name: &str) -> Vec<u32> {
    let device_path = volume_name.trim_end_matches('\\');
    let Ok(handle) = open_handle(device_path, 0, FILE_ATTRIBUTE_NORMAL) else {
        return Vec::new();
    };

    let mut buffer = vec![0u8; DISK_EXTENTS_HEADER_SIZE + MAX_VOLUME_EXTENTS * DISK_EXTENT_SIZE];
    let mut bytes_returned = 0u32;
    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some(buffer.as_mut_ptr().cast()),
            buffer.len() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };
    close_handle(handle);

    if result.is_err() {
        return Vec::new();
    }

    let returned = (bytes_returned as usize).min(buffer.len());
    parse_disk_extent_numbers(&buffer[..returned])
}

/// Reads the disk numbers out of a `VOLUME_DISK_EXTENTS` result: a `u32`
/// count padded to 8 bytes, then 24-byte `DISK_EXTENT` records that each
/// start with a `u32` disk number.
fn parse_disk_extent_numbers(bytes: &[u8]) -> Vec<u32> {
    let Some(count) = bytes.get(0..4) else {
        return Vec::new();
    };
    let count = u32::from_le_bytes(count.try_into().unwrap()) as usize;

    (0..count.min(MAX_VOLUME_EXTENTS))
        .filter_map(|index| {
            let start = DISK_EXTENTS_HEADER_SIZE + index * DISK_EXTENT_SIZE;
            bytes
                .get(start..start + 4)
                .map(|number| u32::from_le_bytes(number.try_into().unwrap()))
        })
        .collect()
}

fn lock_and_dismount_volume(volume_name: &str) -> DeviceResult<LockedVolume> {
    let device_path = volume_name.trim_end_matches('\\');
    let handle = open_handle(
        device_path,
        FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
        FILE_ATTRIBUTE_NORMAL,
    )
    .map_err(|error| volume_lock_error(device_path, format!("could not open it: {error}")))?;
    let volume = LockedVolume { handle };

    // Explorer, antivirus scanners and indexers often hold a volume open for a
    // moment after it is plugged in, so a busy volume is retried briefly.
    let mut attempts = 1;
    while let Err(error) = volume_control(handle, FSCTL_LOCK_VOLUME) {
        if attempts >= VOLUME_LOCK_ATTEMPTS {
            return Err(DeviceError::InUse {
                volume: device_path.to_string(),
                message: error.to_string(),
            });
        }

        attempts += 1;
        std::thread::sleep(VOLUME_LOCK_RETRY_DELAY);
    }

    volume_control(handle, FSCTL_DISMOUNT_VOLUME).map_err(|error| {
        volume_lock_error(device_path, format!("could not dismount it: {error}"))
    })?;

    Ok(volume)
}

fn volume_control(handle: HANDLE, control_code: u32) -> windows::core::Result<()> {
    // Without an OVERLAPPED structure Windows requires a bytes-returned pointer.
    let mut bytes_returned = 0u32;
    unsafe {
        DeviceIoControl(
            handle,
            control_code,
            None,
            0,
            None,
            0,
            Some(&mut bytes_returned),
            None,
        )
    }
}

fn volume_lock_error(device_path: &str, message: String) -> DeviceError {
    DeviceError::Io {
        operation: "lock volume",
        message: format!("{device_path}: {message}"),
    }
}

fn wide_to_string(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    let metadata = query_disk_metadata();
    let mut devices = scan_physical_devices(DEFAULT_SCAN_LIMIT);
    let mut seen_indexes = HashSet::new();

    for device in &mut devices {
        seen_indexes.insert(device.index);
        if let Some(metadata) = metadata.get(&device.index) {
            apply_metadata(device, metadata);
        }
    }

    for metadata in metadata.values() {
        if seen_indexes.contains(&metadata.index) {
            continue;
        }

        devices.push(device_from_metadata(metadata));
    }

    devices.sort_by_key(|device| device.index);
    devices
}

fn scan_physical_devices(limit: u32) -> Vec<PhysicalDevice> {
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
                serial_number: None,
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
                    serial_number: None,
                    contains_system_volume: false,
                    access: DeviceAccess::MetadataOnly,
                    note: Some(format!("read access unavailable: {read_error}")),
                })
            }
            Err(_) => None,
        },
    }
}

fn apply_metadata(device: &mut PhysicalDevice, metadata: &DiskMetadata) {
    device.size_bytes = device.size_bytes.or(metadata.size_bytes);
    device.model = metadata.model.clone();
    device.interface_type = metadata.interface_type.clone();
    device.media_type = metadata.media_type.clone();
    device.pnp_device_id = metadata.pnp_device_id.clone();
    device.serial_number = metadata.serial_number.clone();
    device.contains_system_volume = metadata.contains_system_volume;
}

fn device_from_metadata(metadata: &DiskMetadata) -> PhysicalDevice {
    PhysicalDevice {
        index: metadata.index,
        path: format!(r"\\.\PhysicalDrive{}", metadata.index),
        size_bytes: metadata.size_bytes,
        model: metadata.model.clone(),
        interface_type: metadata.interface_type.clone(),
        media_type: metadata.media_type.clone(),
        pnp_device_id: metadata.pnp_device_id.clone(),
        serial_number: metadata.serial_number.clone(),
        contains_system_volume: metadata.contains_system_volume,
        access: DeviceAccess::MetadataOnly,
        note: Some("listed by Windows disk metadata; direct handle unavailable".to_string()),
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
    serial_number: Option<String>,
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
        serial_number: fields
            .get(7)
            .copied()
            .and_then(optional_field)
            .filter(|serial| !is_placeholder_serial(serial)),
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
) -> windows::core::Result<HANDLE> {
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
        (Clean $containsSystem),
        (Clean $disk.SerialNumber)
    ))
}
"#;

#[cfg(test)]
mod tests {
    use super::{
        close_handle, is_whole_disk, open_handle, parse_disk_extent_numbers, volume_disk_numbers,
        volumes_on_disk, wide_to_string, BlockDevice, WindowsBlockDevice, DISK_EXTENTS_HEADER_SIZE,
        DISK_EXTENT_SIZE, MAX_VOLUME_NAME_CHARS,
    };
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetVolumeNameForVolumeMountPointW, FILE_ATTRIBUTE_NORMAL, FILE_DEVICE_CD_ROM,
        FILE_DEVICE_DISK,
    };
    use windows::Win32::System::Ioctl::{IOCTL_STORAGE_GET_DEVICE_NUMBER, STORAGE_DEVICE_NUMBER};
    use windows::Win32::System::IO::DeviceIoControl;

    #[test]
    fn accepts_only_whole_disks() {
        let disk = |device_type: u32, partition_number: u32| STORAGE_DEVICE_NUMBER {
            DeviceType: device_type,
            DeviceNumber: 2,
            PartitionNumber: partition_number,
        };

        assert!(is_whole_disk(&disk(FILE_DEVICE_DISK.0, 0)));
        assert!(!is_whole_disk(&disk(FILE_DEVICE_DISK.0, 1)));
        assert!(!is_whole_disk(&disk(FILE_DEVICE_CD_ROM.0, 0)));
    }

    #[test]
    fn physical_drive_zero_reports_itself_as_a_whole_disk() {
        let handle = open_handle(r"\\.\PhysicalDrive0", 0, FILE_ATTRIBUTE_NORMAL).unwrap();
        let mut device_number = STORAGE_DEVICE_NUMBER::default();
        let mut bytes_returned = 0u32;
        let result = unsafe {
            DeviceIoControl(
                handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                None,
                0,
                Some((&mut device_number as *mut STORAGE_DEVICE_NUMBER).cast()),
                std::mem::size_of::<STORAGE_DEVICE_NUMBER>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };
        close_handle(handle);

        result.unwrap();
        assert!(is_whole_disk(&device_number));
        assert_eq!(device_number.DeviceNumber, 0);
    }

    #[test]
    fn parses_disk_numbers_from_volume_extents() {
        let mut bytes = vec![0u8; DISK_EXTENTS_HEADER_SIZE + 2 * DISK_EXTENT_SIZE];
        bytes[0..4].copy_from_slice(&2u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
        bytes[32..36].copy_from_slice(&5u32.to_le_bytes());

        assert_eq!(parse_disk_extent_numbers(&bytes), vec![3, 5]);
    }

    #[test]
    fn ignores_extents_missing_from_a_short_buffer() {
        let mut bytes = vec![0u8; DISK_EXTENTS_HEADER_SIZE + DISK_EXTENT_SIZE];
        bytes[0..4].copy_from_slice(&4u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&1u32.to_le_bytes());

        assert_eq!(parse_disk_extent_numbers(&bytes), vec![1]);
        assert!(parse_disk_extent_numbers(&[]).is_empty());
    }

    #[test]
    fn finds_the_system_volume_on_its_disk() {
        let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
        let system_disks = volume_disk_numbers(&format!(r"\\.\{system_drive}"));
        assert!(!system_disks.is_empty(), "system volume reported no disks");

        let mount_point: Vec<u16> = format!("{system_drive}\\")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut name = [0u16; MAX_VOLUME_NAME_CHARS];
        unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(mount_point.as_ptr()), &mut name) }
            .unwrap();
        let system_volume = wide_to_string(&name);

        let volumes = volumes_on_disk(system_disks[0]).unwrap();

        assert!(
            volumes.contains(&system_volume),
            "{system_volume} not among {volumes:?}"
        );
    }

    #[test]
    fn refuses_to_open_something_that_is_not_a_disk() {
        let path =
            std::env::temp_dir().join(format!("osflux-not-a-disk-{}.bin", std::process::id()));
        std::fs::write(&path, b"not a disk").unwrap();

        let result = WindowsBlockDevice::open(&path.to_string_lossy());
        let _ = std::fs::remove_file(&path);

        let error = result
            .err()
            .expect("a regular file must not open as a disk")
            .to_string();
        assert!(
            error.contains("not a whole physical disk")
                || error.contains("could not read the disk number"),
            "{error}"
        );
    }
}
