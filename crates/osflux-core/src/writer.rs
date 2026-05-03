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

pub struct WriteEngine;

impl WriteEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn estimate_buffer_bytes(&self, options: WriteOptions) -> usize {
        options.block_size.saturating_mul(options.queue_depth)
    }
}
