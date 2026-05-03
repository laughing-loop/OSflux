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
    pub access: DeviceAccess,
    pub note: Option<String>,
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
    scan_physical_devices(DEFAULT_SCAN_LIMIT)
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
                    access: DeviceAccess::MetadataOnly,
                    note: Some(format!("read access unavailable: {read_error}")),
                })
            }
            Err(_) => None,
        },
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
