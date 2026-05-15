pub mod chunk_plan;
pub mod image_family;
pub mod image_plan;
pub mod image_report;
pub mod iso;
pub mod iso_filesystem_plan;
pub mod preflight;
pub mod progress;
pub mod target;
pub mod write_command;
pub mod write_intent;
pub mod write_mode_selection;
pub mod write_session;

use osflux_core::device::{self, PhysicalDevice};
use osflux_core::pipeline;

pub use chunk_plan::{
    build_chunk_plan, plan_write_chunks, ChunkPlan, ChunkPlanError, ChunkPlanStatus,
    DEFAULT_CHUNK_SIZE_BYTES,
};
pub use image_family::{
    build_image_family_report, detect_image_family, FamilyEvidence, ImageFamily, ImageFamilyError,
    ImageFamilyReport, ImageFamilyStatus, DEFAULT_FAMILY_SCAN_LIMIT_BYTES,
};
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
    BootStyle, ElToritoBootEntry, ElToritoBootInfo, IsoBootAsset, IsoBootAssetKind,
    IsoBootMetadata, IsoDirectoryEntry, IsoDirectoryMetadata,
};
pub use iso_filesystem_plan::{
    build_iso_filesystem_plan, plan_iso_filesystem, IsoFilesystemCapabilities, IsoFilesystemPlan,
    IsoFilesystemPlanError, IsoFilesystemPlanStatus, IsoFilesystemStrategy,
};
pub use preflight::{
    build_write_preflight, run_write_preflight, WritePreflight, WritePreflightError,
    WritePreflightStatus,
};
pub use progress::{
    build_progress_preview, preview_write_progress, ProgressEvent, ProgressPreview,
    ProgressPreviewStatus,
};
pub use target::{
    assess_target, assess_target_device, TargetAssessment, TargetAssessmentError,
    TargetAssessmentStatus,
};
pub use write_command::{
    build_write_command_plan, prepare_write_command, WriteCommandPlan, WriteCommandStatus,
};
pub use write_intent::{build_write_intent, create_write_intent, WriteIntent, WriteIntentStatus};
pub use write_mode_selection::{
    build_write_mode_selection, select_write_mode, SelectedWriteMode, WriteModeSelection,
    WriteModeSelectionError, WriteModeSelectionStatus,
};
pub use write_session::{
    build_write_session, build_write_session_at, prepare_write_session, WriteSession,
    WriteSessionStatus,
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
