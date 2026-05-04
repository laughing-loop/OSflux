use super::{BlockDevice, DeviceError, DeviceResult, PhysicalDevice};

pub struct MacOsBlockDevice;

impl BlockDevice for MacOsBlockDevice {
    fn open(_path: &str) -> DeviceResult<Self> {
        Err(DeviceError::Unsupported {
            operation: "macOS block device open",
        })
    }

    fn write_at(&self, _offset: u64, _buffer: &[u8]) -> DeviceResult<usize> {
        Err(DeviceError::Unsupported {
            operation: "macOS block device write_at",
        })
    }

    fn flush(&self) -> DeviceResult<()> {
        Err(DeviceError::Unsupported {
            operation: "macOS block device flush",
        })
    }

    fn size(&self) -> DeviceResult<u64> {
        Err(DeviceError::Unsupported {
            operation: "macOS block device size",
        })
    }
}

pub fn list_physical_devices() -> Vec<PhysicalDevice> {
    Vec::new()
}
