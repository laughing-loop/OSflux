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
#[cfg(windows)]
pub use windows::WindowsReadOnlyDevice as ReadOnlyDevice;
#[cfg(not(windows))]
pub use UnsupportedReadOnlyDevice as ReadOnlyDevice;

pub type DeviceResult<T> = Result<T, DeviceError>;

pub trait BlockDevice {
    fn open(path: &str) -> DeviceResult<Self>
    where
        Self: Sized;

    fn write_at(&self, offset: u64, buffer: &[u8]) -> DeviceResult<usize>;

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<usize>;

    fn flush(&self) -> DeviceResult<()>;

    fn size(&self) -> DeviceResult<u64>;

    /// The smallest unit the drive reads and writes. Partition tables count
    /// in these units.
    fn logical_sector_size(&self) -> DeviceResult<u32> {
        Ok(512)
    }
}

/// Reading a drive without changing anything on it. Both a drive opened for
/// writing and one opened only for reading can be read this way, so the same
/// checks run before a write is confirmed and again once the drive is locked.
pub trait DeviceRead {
    /// Fills `buffer` from `offset`, or fails. Offsets and lengths should be
    /// whole sectors.
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<()>;

    fn size_bytes(&self) -> DeviceResult<u64>;

    fn sector_size_bytes(&self) -> DeviceResult<u32>;
}

impl<T: BlockDevice> DeviceRead for T {
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<()> {
        let read = self.read_at(offset, buffer)?;
        if read != buffer.len() {
            return Err(DeviceError::Io {
                operation: "read_at",
                message: format!("read {read} of {} bytes at offset {offset}", buffer.len()),
            });
        }
        Ok(())
    }

    fn size_bytes(&self) -> DeviceResult<u64> {
        self.size()
    }

    fn sector_size_bytes(&self) -> DeviceResult<u32> {
        self.logical_sector_size()
    }
}

/// Opens a whole drive for reading only. Nothing is locked or dismounted, so
/// the drive's volumes stay usable while OSflux looks at it.
pub fn open_read_only(path: &str) -> DeviceResult<ReadOnlyDevice> {
    ReadOnlyDevice::open(path)
}

/// Stands in for a read-only drive on platforms where OSflux cannot open
/// drives yet.
#[cfg(not(windows))]
pub struct UnsupportedReadOnlyDevice;

#[cfg(not(windows))]
impl UnsupportedReadOnlyDevice {
    pub fn open(_path: &str) -> DeviceResult<Self> {
        Err(DeviceError::Unsupported {
            operation: "reading a drive",
        })
    }
}

#[cfg(not(windows))]
impl DeviceRead for UnsupportedReadOnlyDevice {
    fn read_exact_at(&self, _offset: u64, _buffer: &mut [u8]) -> DeviceResult<()> {
        Err(DeviceError::Unsupported {
            operation: "reading a drive",
        })
    }

    fn size_bytes(&self) -> DeviceResult<u64> {
        Err(DeviceError::Unsupported {
            operation: "reading a drive",
        })
    }

    fn sector_size_bytes(&self) -> DeviceResult<u32> {
        Err(DeviceError::Unsupported {
            operation: "reading a drive",
        })
    }
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
    /// Another program kept one of the drive's volumes open, so it could
    /// not be locked for writing.
    InUse {
        volume: String,
        message: String,
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
            Self::InUse { volume, message } => write!(
                formatter,
                "the drive's volume {volume} is in use by another program or window ({message})"
            ),
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
    /// Only drives that report removable media, such as USB sticks and SD
    /// cards, are write candidates. A USB interface alone is not enough:
    /// external hard disks and SSDs connect over USB too.
    pub fn is_likely_removable_target(&self) -> bool {
        self.media_type_contains("removable")
    }

    /// External hard disks and SSDs usually hold the user's data, so they are
    /// never write targets.
    pub fn is_external_disk(&self) -> bool {
        self.media_type_contains("external")
    }

    pub fn safety_label(&self) -> &'static str {
        if self.contains_system_volume {
            "system-drive"
        } else if self.is_external_disk() {
            "external-disk"
        } else if self.is_likely_removable_target() {
            "candidate"
        } else {
            "internal-or-unknown"
        }
    }

    fn media_type_contains(&self, needle: &str) -> bool {
        self.media_type
            .as_deref()
            .is_some_and(|value| value.to_ascii_lowercase().contains(needle))
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

/// USB-to-SATA adapters often report a serial number of all zeros instead of
/// the drive's real one. Every drive in the same kind of enclosure shares that
/// value, so it is treated as not reported.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_placeholder_serial(value: &str) -> bool {
    value.chars().all(|character| character == '0')
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

#[cfg(test)]
mod tests {
    use super::{is_placeholder_serial, DeviceAccess, PhysicalDevice};

    #[test]
    fn labels_external_ssd_over_usb_as_external_disk() {
        let device = device("USB", "External hard disk media", false);

        assert!(device.is_external_disk());
        assert!(!device.is_likely_removable_target());
        assert_eq!(device.safety_label(), "external-disk");
    }

    #[test]
    fn labels_usb_stick_as_candidate() {
        let device = device("USB", "Removable Media", false);

        assert!(device.is_likely_removable_target());
        assert_eq!(device.safety_label(), "candidate");
    }

    #[test]
    fn does_not_treat_usb_interface_alone_as_removable() {
        let device = device("USB", "Fixed hard disk media", false);

        assert!(!device.is_likely_removable_target());
        assert_eq!(device.safety_label(), "internal-or-unknown");
    }

    #[test]
    fn system_drive_label_takes_priority() {
        let device = device("IDE", "Fixed hard disk media", true);

        assert_eq!(device.safety_label(), "system-drive");
    }

    #[test]
    fn treats_all_zero_serial_as_placeholder() {
        assert!(is_placeholder_serial("0000000000000000"));
        assert!(!is_placeholder_serial("WD-WCC6Z6FLN33L"));
        assert!(!is_placeholder_serial("TPBF2307030050200733"));
    }

    fn device(interface_type: &str, media_type: &str, system: bool) -> PhysicalDevice {
        PhysicalDevice {
            index: 2,
            path: r"\\.\PhysicalDrive2".to_string(),
            size_bytes: Some(32 * 1024 * 1024),
            model: Some("Test Drive".to_string()),
            interface_type: Some(interface_type.to_string()),
            media_type: Some(media_type.to_string()),
            pnp_device_id: None,
            serial_number: None,
            contains_system_volume: system,
            access: DeviceAccess::Readable,
            note: None,
        }
    }
}
