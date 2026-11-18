use std::fmt;
use std::path::Path;

use crate::chunk_plan::{plan_write_chunks, ChunkPlan, ChunkPlanError};
use crate::image_plan::{
    plan_image, ImagePlan, ImagePlanError, ImagePlanStatus, ImageWriteStrategy,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteModeSelection {
    pub status: WriteModeSelectionStatus,
    pub reason: String,
    pub selected_mode: Option<SelectedWriteMode>,
    pub image_source: String,
    pub image_kind: String,
    pub image_status: ImagePlanStatus,
    pub image_write_strategy: ImageWriteStrategy,
    pub session_id: String,
    pub target_index: u32,
    pub target_path: String,
    pub chunk_count: u64,
    pub planned_bytes: u64,
    pub dry_run: bool,
    pub write_access_attempted: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl WriteModeSelection {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"selectedMode\": {},\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"imageKind\": \"{}\",\n",
                "  \"imageStatus\": \"{}\",\n",
                "  \"imageWriteStrategy\": \"{}\",\n",
                "  \"sessionId\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"chunkCount\": {},\n",
                "  \"plannedBytes\": {},\n",
                "  \"dryRun\": {},\n",
                "  \"writeAccessAttempted\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            optional_mode_json(self.selected_mode),
            escape_json_string(&self.image_source),
            escape_json_string(&self.image_kind),
            self.image_status.as_str(),
            self.image_write_strategy.as_str(),
            escape_json_string(&self.session_id),
            self.target_index,
            escape_json_string(&self.target_path),
            self.chunk_count,
            self.planned_bytes,
            self.dry_run,
            self.write_access_attempted,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteModeSelectionStatus {
    Selected,
    NeedsInspection,
    Blocked,
}

impl WriteModeSelectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::NeedsInspection => "needs-inspection",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for WriteModeSelectionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedWriteMode {
    IsoFilesystemBootSetup,
    RawBlockWrite,
}

impl SelectedWriteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IsoFilesystemBootSetup => "iso-filesystem-boot-setup",
            Self::RawBlockWrite => "raw-block-write",
        }
    }
}

impl fmt::Display for SelectedWriteMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub enum WriteModeSelectionError {
    Image(ImagePlanError),
    ChunkPlan(ChunkPlanError),
}

impl fmt::Display for WriteModeSelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => write!(formatter, "image planning failed: {error}"),
            Self::ChunkPlan(error) => write!(formatter, "chunk planning failed: {error}"),
        }
    }
}

impl std::error::Error for WriteModeSelectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Image(error) => Some(error),
            Self::ChunkPlan(error) => Some(error),
        }
    }
}

pub fn select_write_mode(
    image_path: impl AsRef<Path>,
    target_index: u32,
    confirmation_phrase: Option<&str>,
    chunk_size_bytes: u64,
) -> Result<WriteModeSelection, WriteModeSelectionError> {
    let image_path = image_path.as_ref();
    let image_plan = plan_image(image_path).map_err(WriteModeSelectionError::Image)?;
    let chunk_plan = plan_write_chunks(
        image_path,
        target_index,
        confirmation_phrase,
        chunk_size_bytes,
    )
    .map_err(WriteModeSelectionError::ChunkPlan)?;

    Ok(build_write_mode_selection(&image_plan, &chunk_plan))
}

pub fn build_write_mode_selection(
    image_plan: &ImagePlan,
    chunk_plan: &ChunkPlan,
) -> WriteModeSelection {
    let selected_mode = selected_mode_for(image_plan);
    let status = classify_mode_selection(image_plan, chunk_plan, selected_mode);
    let mut notes = Vec::new();
    let mut warnings = Vec::new();

    notes.push("Write mode selection is read-only; no target writes were attempted.".to_string());
    notes.push(format!(
        "Image strategy candidate: {}.",
        image_plan.write_strategy.as_str()
    ));
    notes.push(format!(
        "Chunk plan contributes {} planned chunk(s).",
        chunk_plan.chunk_count
    ));

    warnings.extend(
        image_plan
            .warnings
            .iter()
            .map(|warning| format!("Image: {warning}")),
    );
    warnings.extend(
        chunk_plan
            .warnings
            .iter()
            .map(|warning| format!("Chunk plan: {warning}")),
    );

    match status {
        WriteModeSelectionStatus::Selected => {
            notes.push("Selected mode can be used by a future write executor.".to_string());
        }
        WriteModeSelectionStatus::NeedsInspection => {
            warnings.push(
                "Mode selection requires deeper image inspection before execution.".to_string(),
            );
        }
        WriteModeSelectionStatus::Blocked => {
            warnings
                .push("Mode selection is blocked and cannot reach write execution.".to_string());
        }
    }

    WriteModeSelection {
        status,
        reason: mode_selection_reason(image_plan, chunk_plan, selected_mode),
        selected_mode: if status == WriteModeSelectionStatus::Selected {
            selected_mode
        } else {
            None
        },
        image_source: image_plan.source.display().to_string(),
        image_kind: image_plan.kind.as_str().to_string(),
        image_status: image_plan.status,
        image_write_strategy: image_plan.write_strategy,
        session_id: chunk_plan.session_id.clone(),
        target_index: chunk_plan.target_index,
        target_path: chunk_plan.target_path.clone(),
        chunk_count: chunk_plan.chunk_count,
        planned_bytes: chunk_plan.planned_bytes,
        dry_run: true,
        write_access_attempted: false,
        notes,
        warnings,
    }
}

fn classify_mode_selection(
    image_plan: &ImagePlan,
    chunk_plan: &ChunkPlan,
    selected_mode: Option<SelectedWriteMode>,
) -> WriteModeSelectionStatus {
    if image_plan.status == ImagePlanStatus::Blocked || chunk_plan.planned_bytes == 0 {
        return WriteModeSelectionStatus::Blocked;
    }

    if chunk_plan.write_access_attempted || !chunk_plan.dry_run {
        return WriteModeSelectionStatus::Blocked;
    }

    if image_plan.status == ImagePlanStatus::NeedsInspection {
        return WriteModeSelectionStatus::NeedsInspection;
    }

    if selected_mode.is_none() {
        return WriteModeSelectionStatus::Blocked;
    }

    WriteModeSelectionStatus::Selected
}

fn selected_mode_for(image_plan: &ImagePlan) -> Option<SelectedWriteMode> {
    match image_plan.write_strategy {
        ImageWriteStrategy::IsoFileSystemBootSetup => {
            Some(SelectedWriteMode::IsoFilesystemBootSetup)
        }
        ImageWriteStrategy::RawBlockWrite | ImageWriteStrategy::HybridIsoRawWrite => {
            Some(SelectedWriteMode::RawBlockWrite)
        }
        ImageWriteStrategy::IsoNeedsDeeperInspection | ImageWriteStrategy::Unknown => None,
    }
}

fn mode_selection_reason(
    image_plan: &ImagePlan,
    chunk_plan: &ChunkPlan,
    selected_mode: Option<SelectedWriteMode>,
) -> String {
    if image_plan.status == ImagePlanStatus::Blocked {
        return "write mode selection blocked by image plan".to_string();
    }

    // Pass on why the chunk plan was blocked, such as a mismatched
    // confirmation phrase, rather than only that it was.
    if chunk_plan.planned_bytes == 0 {
        return format!("write mode selection blocked: {}", chunk_plan.reason);
    }

    if chunk_plan.write_access_attempted || !chunk_plan.dry_run {
        return "write mode selection blocked because chunk plan is not read-only".to_string();
    }

    if image_plan.status == ImagePlanStatus::NeedsInspection {
        return "write mode selection needs deeper image inspection".to_string();
    }

    let reason = match selected_mode {
        Some(SelectedWriteMode::IsoFilesystemBootSetup) => {
            "selected filesystem boot setup mode for validated ISO metadata"
        }
        Some(SelectedWriteMode::RawBlockWrite)
            if image_plan.write_strategy == ImageWriteStrategy::HybridIsoRawWrite =>
        {
            "selected raw block write mode for hybrid ISO"
        }
        Some(SelectedWriteMode::RawBlockWrite) => {
            "selected raw block write mode for raw disk image"
        }
        None => "write mode selection blocked because no mode is available",
    };
    reason.to_string()
}

fn optional_mode_json(value: Option<SelectedWriteMode>) -> String {
    match value {
        Some(value) => format!("\"{}\"", value.as_str()),
        None => "null".to_string(),
    }
}

fn string_array_json(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("\"{}\"", escape_json_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
}

fn escape_json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                write!(escaped, "\\u{:04x}", character as u32)
                    .expect("writing JSON escape should not fail");
            }
            character => escaped.push(character),
        }
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::{build_write_mode_selection, SelectedWriteMode, WriteModeSelectionStatus};
    use crate::chunk_plan::{ChunkPlan, ChunkPlanStatus};
    use crate::image_plan::{
        DetectionMethod, ImageKind, ImagePlan, ImagePlanStatus, ImageWriteStrategy, WriteMode,
    };
    use crate::write_session::WriteSessionStatus;

    #[test]
    fn selects_raw_block_write_for_ready_raw_image() {
        let selection = build_write_mode_selection(
            &image(
                ImageKind::RawDiskImage,
                ImagePlanStatus::Ready,
                ImageWriteStrategy::RawBlockWrite,
            ),
            &chunk_plan(ChunkPlanStatus::Planned),
        );

        assert_eq!(selection.status, WriteModeSelectionStatus::Selected);
        assert_eq!(
            selection.selected_mode,
            Some(SelectedWriteMode::RawBlockWrite)
        );
        assert!(!selection.write_access_attempted);
    }

    #[test]
    fn selects_iso_filesystem_boot_setup_for_valid_iso() {
        let selection = build_write_mode_selection(
            &image(
                ImageKind::Iso,
                ImagePlanStatus::Ready,
                ImageWriteStrategy::IsoFileSystemBootSetup,
            ),
            &chunk_plan(ChunkPlanStatus::Planned),
        );

        assert_eq!(selection.status, WriteModeSelectionStatus::Selected);
        assert_eq!(
            selection.selected_mode,
            Some(SelectedWriteMode::IsoFilesystemBootSetup)
        );
    }

    #[test]
    fn needs_inspection_when_image_needs_inspection() {
        let selection = build_write_mode_selection(
            &image(
                ImageKind::Iso,
                ImagePlanStatus::NeedsInspection,
                ImageWriteStrategy::IsoNeedsDeeperInspection,
            ),
            &chunk_plan(ChunkPlanStatus::Planned),
        );

        assert_eq!(selection.status, WriteModeSelectionStatus::NeedsInspection);
        assert_eq!(selection.selected_mode, None);
    }

    #[test]
    fn blocks_when_chunk_plan_is_blocked() {
        let selection = build_write_mode_selection(
            &image(
                ImageKind::RawDiskImage,
                ImagePlanStatus::Ready,
                ImageWriteStrategy::RawBlockWrite,
            ),
            &chunk_plan(ChunkPlanStatus::Blocked),
        );

        assert_eq!(selection.status, WriteModeSelectionStatus::Blocked);
        assert_eq!(selection.selected_mode, None);
        assert_eq!(selection.reason, "write mode selection blocked: blocked");
    }

    #[test]
    fn selects_raw_block_write_for_hybrid_iso() {
        let selection = build_write_mode_selection(
            &image(
                ImageKind::Iso,
                ImagePlanStatus::Ready,
                ImageWriteStrategy::HybridIsoRawWrite,
            ),
            &chunk_plan(ChunkPlanStatus::Planned),
        );

        assert_eq!(selection.status, WriteModeSelectionStatus::Selected);
        assert_eq!(
            selection.selected_mode,
            Some(SelectedWriteMode::RawBlockWrite)
        );
        assert_eq!(
            selection.reason,
            "selected raw block write mode for hybrid ISO"
        );
    }

    #[test]
    fn serializes_mode_selection_as_json() {
        let selection = build_write_mode_selection(
            &image(
                ImageKind::RawDiskImage,
                ImagePlanStatus::Ready,
                ImageWriteStrategy::RawBlockWrite,
            ),
            &chunk_plan(ChunkPlanStatus::Planned),
        );
        let json = selection.to_json_pretty();

        assert!(json.contains("\"status\": \"selected\""));
        assert!(json.contains("\"selectedMode\": \"raw-block-write\""));
        assert!(json.contains("\"writeAccessAttempted\": false"));
    }

    fn image(
        kind: ImageKind,
        status: ImagePlanStatus,
        write_strategy: ImageWriteStrategy,
    ) -> ImagePlan {
        ImagePlan {
            source: "C:\\images\\sample.img".into(),
            file_name: "sample.img".to_string(),
            size_bytes: 8 * 1024 * 1024,
            status,
            status_reason: status.as_str().to_string(),
            kind,
            detection_method: DetectionMethod::Signature,
            signatures: Vec::new(),
            hybrid: false,
            boot_metadata: None,
            write_mode: match kind {
                ImageKind::Iso => WriteMode::FileSystem,
                ImageKind::RawDiskImage => WriteMode::Raw,
                ImageKind::Unknown => WriteMode::Unknown,
            },
            write_strategy,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn chunk_plan(status: ChunkPlanStatus) -> ChunkPlan {
        ChunkPlan {
            status,
            reason: status.as_str().to_string(),
            session_id: "OSFLUX-SESSION-1700000000-2-TEST".to_string(),
            session_status: if status == ChunkPlanStatus::Planned {
                WriteSessionStatus::Prepared
            } else {
                WriteSessionStatus::Blocked
            },
            image_source: "C:\\images\\sample.img".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            image_size_bytes: if status == ChunkPlanStatus::Planned {
                8 * 1024 * 1024
            } else {
                0
            },
            chunk_size_bytes: 4 * 1024 * 1024,
            chunk_count: if status == ChunkPlanStatus::Planned {
                2
            } else {
                0
            },
            final_chunk_size_bytes: if status == ChunkPlanStatus::Planned {
                4 * 1024 * 1024
            } else {
                0
            },
            planned_bytes: if status == ChunkPlanStatus::Planned {
                8 * 1024 * 1024
            } else {
                0
            },
            dry_run: true,
            write_access_attempted: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
