use osflux_core::pipeline;

#[derive(Debug, Clone, Copy)]
pub struct EngineConfig {
    pub block_count: usize,
    pub block_size: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            block_count: 8,
            block_size: 1024 * 1024,
        }
    }
}

pub struct Engine {
    config: EngineConfig,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn start(&self) {
        println!("OSflux Engine starting...");
        let processed = pipeline::run_pipeline(self.config.block_count, self.config.block_size);
        println!("OSflux Core pipeline processed {processed} block(s).");
    }
}
