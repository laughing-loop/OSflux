pub mod answer_file;
pub mod audit;
pub mod checksum;
pub mod chunk_plan;
pub mod clock;
mod crc;
pub mod drive_contents;
pub mod fat32;
pub mod file_copy;
mod fingerprint;
pub mod image_family;
pub mod image_io;
pub mod image_plan;
pub mod image_report;
pub mod iso;
pub mod iso_filesystem_plan;
mod json;
pub mod preflight;
pub mod progress;
pub mod raw_write;
pub mod safety_check;
mod size_label;
pub mod target;
#[cfg(test)]
mod test_support;
pub mod udf;
pub mod wim_split;
pub mod write_authorization;
pub mod write_command;
pub mod write_intent;
pub mod write_mode_selection;
pub mod write_session;

use osflux_core::device::{self, PhysicalDevice};
use osflux_core::pipeline;

pub use answer_file::{
    validate_account_name, Architecture, RegionSettings, SetupOptions, ANSWER_FILE_NAME,
};
pub use audit::{AuditEvent, AuditEventKind, AuditLog};
pub use checksum::{Sha256Hex, Sha256ParseError};
pub use chunk_plan::{
    build_chunk_plan, plan_write_chunks, ChunkPlan, ChunkPlanError, ChunkPlanStatus,
    DEFAULT_CHUNK_SIZE_BYTES,
};
pub use clock::{format_unix_rfc3339, format_unix_utc, now_unix_seconds};
pub use drive_contents::{
    inspect_device_contents, inspect_drive_contents, DriveContents, DriveContentsState,
    DrivePartition, FilesystemKind, PartitionScheme,
};
pub use file_copy::{
    lay_out_uefi_fat32, summarize_iso_files, FileCopyError, IsoFilesSummary, SplitFile,
    UefiFat32Layout,
};
pub use image_family::{
    build_image_family_report, detect_image_family, detect_plan_family, FamilyEvidence,
    ImageFamily, ImageFamilyError, ImageFamilyReport, ImageFamilyStatus,
    DEFAULT_FAMILY_SCAN_LIMIT_BYTES,
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
pub use osflux_core::device::{DeviceAccess, DeviceError};
pub use osflux_core::privilege::is_elevated;
pub use osflux_core::region::read_region_settings;
pub use preflight::{
    build_write_preflight, run_write_preflight, WritePreflight, WritePreflightError,
    WritePreflightStatus,
};
pub use progress::{
    build_progress_preview, preview_write_progress, ProgressEvent, ProgressPreview,
    ProgressPreviewStatus,
};
pub use raw_write::{
    build_raw_write_plan, execute_raw_write, prepare_raw_write, write_image_to_target,
    RawWriteError, RawWritePhase, RawWritePlan, RawWriteProgress, RawWriteReport, Verification,
    WriteLayout, MAX_RAW_WRITE_CHUNK_BYTES, RAW_WRITE_ALIGNMENT_BYTES,
};
pub use safety_check::{
    build_safety_check, run_safety_check, SafetyCheck, SafetyCheckError, SafetyCheckItem,
    SafetyCheckKind, SafetyCheckState,
};
pub use size_label::size_label;
pub use target::{
    assess_all_targets, assess_target, assess_target_device, with_drive_contents, TargetAssessment,
    TargetAssessmentError, TargetAssessmentStatus,
};
pub use write_authorization::{
    record_write_authorization, state_directory, AuthorizationProblem, AuthorizationStore,
    AuthorizationStoreError, ImageIdentity, WriteAuthorization, AUTHORIZATION_LIFETIME_SECONDS,
    STATE_DIR_ENV,
};
pub use write_command::{
    build_write_command_plan, prepare_write_command, WriteCommandPlan, WriteCommandStatus,
};
pub use write_intent::{
    build_write_intent, confirmation_phrase, contents_acknowledgement, create_write_intent,
    match_confirmation, match_supplied_confirmation, ConfirmationForm, SuppliedConfirmation,
    WriteIntent, WriteIntentStatus,
};
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
