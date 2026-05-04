use crate::device::{BlockDevice, DeviceResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsWriteStrategy;

impl<D: BlockDevice> super::WriteStrategy<D> for WindowsWriteStrategy {
    fn name(&self) -> &'static str {
        "windows-overlapped"
    }

    fn write_at(&self, device: &D, offset: u64, buffer: &[u8]) -> DeviceResult<usize> {
        device.write_at(offset, buffer)
    }

    fn flush(&self, device: &D) -> DeviceResult<()> {
        device.flush()
    }
}
