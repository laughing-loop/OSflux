use crate::device::{BlockDevice, DeviceResult};

#[derive(Debug, Default, Clone, Copy)]
pub struct UnixWriteStrategy;

impl<D: BlockDevice> super::WriteStrategy<D> for UnixWriteStrategy {
    fn name(&self) -> &'static str {
        "unix-pwrite"
    }

    fn write_at(&self, device: &D, offset: u64, buffer: &[u8]) -> DeviceResult<usize> {
        device.write_at(offset, buffer)
    }

    fn flush(&self, device: &D) -> DeviceResult<()> {
        device.flush()
    }
}
