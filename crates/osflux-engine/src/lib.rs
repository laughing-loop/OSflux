pub mod image_plan;
pub mod image_report;
pub mod iso;
pub mod target;

use osflux_core::device::{self, PhysicalDevice};
use osflux_core::pipeline;

pub use image_plan::{
    plan_image, DetectionMethod, ImageKind, ImagePlan, ImagePlanError, ImagePlanStatus,
    ImageSignature, ImageWriteStrategy, WriteMode,
};
pub use image_report::{
    build_image_plan_report, plan_image_report, PlanReport, PlanReportField, PlanReportItem,
    PlanReportSection,
};
pub use iso::{
    BootCatalogPlatform, BootEmulation, BootEntrySource, BootImageRange, BootImageRangeStatus,
    BootStyle, ElToritoBootEntry, ElToritoBootInfo, IsoBootMetadata,
};
pub use target::{
    assess_target, assess_target_device, TargetAssessment, TargetAssessmentError,
    TargetAssessmentStatus,
};

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

pub fn list_devices() -> Vec<PhysicalDevice> {
    device::list_physical_devices()
}
