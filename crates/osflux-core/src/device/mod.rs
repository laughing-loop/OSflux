use std::error::Error;
use std::fmt;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::LinuxBlockDevice as Device;
#[cfg(target_os = "linux")]
pub use linux::LinuxBlockDevice as NativeBlockDevice;
#[cfg(target_os = "macos")]
pub use macos::MacOsBlockDevice as Device;
#[cfg(target_os = "macos")]
pub use macos::MacOsBlockDevice as NativeBlockDevice;
#[cfg(windows)]
pub use windows::WindowsBlockDevice as Device;
#[cfg(windows)]
pub use windows::WindowsBlockDevice as NativeBlockDevice;

pub type DeviceResult<T> = Result<T, DeviceError>;

pub trait BlockDevice {
    fn open(path: &str) -> DeviceResult<Self>
    where
        Self: Sized;

    fn write_at(&self, offset: u64, buffer: &[u8]) -> DeviceResult<usize>;

    fn flush(&self) -> DeviceResult<()>;

    fn size(&self) -> DeviceResult<u64>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    Open {
        path: String,
        message: String,
    },
    Io {
        operation: &'static str,
        message: String,
    },
    Unsupported {
        operation: &'static str,
    },
}

impl fmt::Display for DeviceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { path, message } => {
                write!(formatter, "could not open block device {path}: {message}")
            }
            Self::Io { operation, message } => {
                write!(formatter, "{operation} failed: {message}")
            }
            Self::Unsupported { operation } => {
                write!(
                    formatter,
                    "{operation} is not implemented on this platform yet"
                )
            }
        }
    }
}

impl Error for DeviceError {}

#[derive(Debug, Clone)]
pub struct PhysicalDevice {
    pub index: u32,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub model: Option<String>,
    pub interface_type: Option<String>,
    pub media_type: Option<String>,
    pub pnp_device_id: Option<String>,
    pub serial_number: Option<String>,
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

#[cfg(windows)]
pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    windows::list_physical_devices()
}

#[cfg(target_os = "linux")]
pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    linux::list_physical_devices()
}

#[cfg(target_os = "macos")]
pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    macos::list_physical_devices()
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    Vec::new()
}
