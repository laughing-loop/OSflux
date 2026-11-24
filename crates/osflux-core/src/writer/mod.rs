use crate::device::{BlockDevice, DeviceResult};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::UnixWriteStrategy as PlatformWriteStrategy;
#[cfg(windows)]
pub use windows::WindowsWriteStrategy as PlatformWriteStrategy;

#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    pub block_size: usize,
    pub queue_depth: usize,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            block_size: 1024 * 1024,
            queue_depth: 4,
        }
    }
}

pub trait WriteStrategy<D: BlockDevice> {
    fn name(&self) -> &'static str;

    fn write_at(&self, device: &D, offset: u64, buffer: &[u8]) -> DeviceResult<usize>;

    fn flush(&self, device: &D) -> DeviceResult<()>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DirectWriteStrategy;

impl<D: BlockDevice> WriteStrategy<D> for DirectWriteStrategy {
    fn name(&self) -> &'static str {
        "direct"
    }

    fn write_at(&self, device: &D, offset: u64, buffer: &[u8]) -> DeviceResult<usize> {
        device.write_at(offset, buffer)
    }

    fn flush(&self, device: &D) -> DeviceResult<()> {
        device.flush()
    }
}

pub struct WriteEngine<S = PlatformWriteStrategy> {
    strategy: S,
}

impl WriteEngine {
    // The platform strategy is a unit struct on some platforms only, so it
    // is built the same way everywhere.
    #[allow(clippy::default_constructed_unit_structs)]
    pub fn new() -> Self {
        Self {
            strategy: PlatformWriteStrategy::default(),
        }
    }
}

impl Default for WriteEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> WriteEngine<S> {
    pub fn with_strategy(strategy: S) -> Self {
        Self { strategy }
    }

    pub fn strategy(&self) -> &S {
        &self.strategy
    }

    pub fn estimate_buffer_bytes(&self, options: WriteOptions) -> usize {
        options.block_size.saturating_mul(options.queue_depth)
    }
}

#[cfg(test)]
mod tests {
    use super::{WriteEngine, WriteOptions};

    #[test]
    fn estimates_buffer_bytes_from_block_size_and_queue_depth() {
        let engine = WriteEngine::new();

        let estimate = engine.estimate_buffer_bytes(WriteOptions {
            block_size: 1024,
            queue_depth: 8,
        });

        assert_eq!(estimate, 8192);
    }
}
