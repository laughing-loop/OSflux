use std::fmt;
use std::path::Path;

use crate::drive_contents::DriveContents;
use crate::image_plan::{plan_image, ImagePlan, ImagePlanError, ImagePlanStatus};
use crate::target::{
    assess_target, TargetAssessment, TargetAssessmentError, TargetAssessmentStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritePreflight {
    pub status: WritePreflightStatus,
    pub reason: String,
    pub image_source: String,
    pub image_status: ImagePlanStatus,
    pub image_status_reason: String,
    pub image_write_strategy: String,
    pub target_index: u32,
    pub target_path: String,
    pub target_model: Option<String>,
    pub target_size_bytes: Option<u64>,
    pub target_identity_code: Option<String>,
    pub target_status: TargetAssessmentStatus,
    pub target_reason: String,
    /// What the target holds, when it could be read. A confirmation that
    /// names what will be lost is built from it.
    pub target_contents: Option<DriveContents>,
    pub requires_confirmation: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl WritePreflight {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"imageStatus\": \"{}\",\n",
                "  \"imageStatusReason\": \"{}\",\n",
                "  \"imageWriteStrategy\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"targetModel\": {},\n",
                "  \"targetSizeBytes\": {},\n",
                "  \"targetIdentityCode\": {},\n",
                "  \"targetStatus\": \"{}\",\n",
                "  \"targetReason\": \"{}\",\n",
                "  \"requiresConfirmation\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            escape_json_string(&self.image_source),
            self.image_status.as_str(),
            escape_json_string(&self.image_status_reason),
            escape_json_string(&self.image_write_strategy),
            self.target_index,
            escape_json_string(&self.target_path),
            optional_string_json(self.target_model.as_deref()),
            optional_number_json(self.target_size_bytes),
            optional_string_json(self.target_identity_code.as_deref()),
            self.target_status.as_str(),
            escape_json_string(&self.target_reason),
            self.requires_confirmation,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePreflightStatus {
    Ready,
    NeedsConfirmation,
    NeedsInspection,
    Blocked,
}

impl WritePreflightStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsConfirmation => "needs-confirmation",
            Self::NeedsInspection => "needs-inspection",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug)]
pub enum WritePreflightError {
    Image(ImagePlanError),
    Target(TargetAssessmentError),
}

impl fmt::Display for WritePreflightError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => write!(formatter, "image preflight failed: {error}"),
            Self::Target(error) => write!(formatter, "target preflight failed: {error}"),
        }
    }
}

impl std::error::Error for WritePreflightError {}

pub fn run_write_preflight(
    image_path: impl AsRef<Path>,
    target_index: u32,
) -> Result<WritePreflight, WritePreflightError> {
    let image_plan = plan_image(image_path).map_err(WritePreflightError::Image)?;
    let target = assess_target(target_index).map_err(WritePreflightError::Target)?;

    Ok(build_write_preflight(&image_plan, &target))
}

pub fn build_write_preflight(image_plan: &ImagePlan, target: &TargetAssessment) -> WritePreflight {
    let (status, reason) = classify_preflight(image_plan, target);
    let mut notes = Vec::new();
    let mut warnings = Vec::new();

    notes.push("Preflight is read-only; no device writes were attempted.".to_string());
    notes.push(format!(
        "Image strategy: {}.",
        image_plan.write_strategy.as_str()
    ));
    notes.push(format!("Target path: {}.", target.path));

    warnings.extend(
        image_plan
            .warnings
            .iter()
            .map(|warning| format!("Image: {warning}")),
    );
    warnings.extend(
        target
            .warnings
            .iter()
            .map(|warning| format!("Target: {warning}")),
    );

    if let Some(target_size) = target.size_bytes {
        if image_plan.size_bytes > target_size {
            warnings.push(format!(
                "Image needs {} bytes but the target only holds {} bytes.",
                image_plan.size_bytes, target_size
            ));
        }
    }

    if status == WritePreflightStatus::NeedsConfirmation {
        warnings.push("Explicit target confirmation is still required before writing.".to_string());
    }

    WritePreflight {
        status,
        reason: reason.to_string(),
        image_source: image_plan.source.display().to_string(),
        image_status: image_plan.status,
        image_status_reason: image_plan.status_reason.clone(),
        image_write_strategy: image_plan.write_strategy.as_str().to_string(),
        target_index: target.index,
        target_path: target.path.clone(),
        target_model: target.model.clone(),
        target_size_bytes: target.size_bytes,
        target_identity_code: target.identity_code.clone(),
        target_status: target.status,
        target_reason: target.reason.clone(),
        target_contents: target.contents.clone(),
        requires_confirmation: target.requires_confirmation,
        notes,
        warnings,
    }
}

fn classify_preflight(
    image_plan: &ImagePlan,
    target: &TargetAssessment,
) -> (WritePreflightStatus, &'static str) {
    if image_plan.status == ImagePlanStatus::Blocked {
        return (
            WritePreflightStatus::Blocked,
            "image plan is blocked and cannot enter a write workflow",
        );
    }

    if target.status == TargetAssessmentStatus::Blocked {
        return (
            WritePreflightStatus::Blocked,
            "target device is blocked and cannot be used",
        );
    }

    if image_exceeds_target(image_plan, target) {
        return (
            WritePreflightStatus::Blocked,
            "image is larger than the target device",
        );
    }

    if image_plan.status == ImagePlanStatus::NeedsInspection {
        return (
            WritePreflightStatus::NeedsInspection,
            "image needs deeper inspection before write preflight can pass",
        );
    }

    if target.status == TargetAssessmentStatus::NeedsConfirmation {
        return (
            WritePreflightStatus::NeedsConfirmation,
            "image and target passed read-only checks; explicit confirmation is required",
        );
    }

    (
        WritePreflightStatus::Ready,
        "image and target passed read-only checks",
    )
}

fn image_exceeds_target(image_plan: &ImagePlan, target: &TargetAssessment) -> bool {
    target
        .size_bytes
        .is_some_and(|target_size| image_plan.size_bytes > target_size)
}

fn optional_string_json(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", escape_json_string(value)))
        .unwrap_or_else(|| "null".to_string())
}

fn optional_number_json(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
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
    use super::{build_write_preflight, WritePreflightStatus};
    use crate::image_plan::{
        DetectionMethod, ImageKind, ImagePlan, ImagePlanStatus, ImageWriteStrategy, WriteMode,
    };
    use crate::target::{TargetAssessment, TargetAssessmentStatus};
    use osflux_core::device::DeviceAccess;

    #[test]
    fn blocks_when_image_is_blocked() {
        let preflight = build_write_preflight(&image(ImagePlanStatus::Blocked), &target(false));

        assert_eq!(preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(
            preflight.reason,
            "image plan is blocked and cannot enter a write workflow"
        );
    }

    #[test]
    fn blocks_when_target_is_blocked() {
        let preflight = build_write_preflight(&image(ImagePlanStatus::Ready), &target(true));

        assert_eq!(preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(
            preflight.reason,
            "target device is blocked and cannot be used"
        );
    }

    #[test]
    fn needs_inspection_when_image_needs_inspection() {
        let preflight =
            build_write_preflight(&image(ImagePlanStatus::NeedsInspection), &target(false));

        assert_eq!(preflight.status, WritePreflightStatus::NeedsInspection);
    }

    #[test]
    fn needs_confirmation_for_ready_image_and_candidate_target() {
        let preflight = build_write_preflight(&image(ImagePlanStatus::Ready), &target(false));

        assert_eq!(preflight.status, WritePreflightStatus::NeedsConfirmation);
        assert!(preflight.requires_confirmation);
    }

    #[test]
    fn blocks_when_image_is_larger_than_target() {
        let mut image = image(ImagePlanStatus::Ready);
        image.size_bytes = 64 * 1024 * 1024;

        let preflight = build_write_preflight(&image, &target(false));

        assert_eq!(preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(preflight.reason, "image is larger than the target device");
        assert!(preflight
            .warnings
            .iter()
            .any(|warning| warning.contains("target only holds 33554432 bytes")));
    }

    #[test]
    fn allows_image_that_exactly_fills_target() {
        let mut image = image(ImagePlanStatus::Ready);
        image.size_bytes = 32 * 1024 * 1024;

        let preflight = build_write_preflight(&image, &target(false));

        assert_eq!(preflight.status, WritePreflightStatus::NeedsConfirmation);
    }

    #[test]
    fn serializes_preflight_as_json() {
        let preflight = build_write_preflight(&image(ImagePlanStatus::Ready), &target(false));
        let json = preflight.to_json_pretty();

        assert!(json.contains("\"status\": \"needs-confirmation\""));
        assert!(json.contains("\"targetIndex\": 2"));
        assert!(json.contains("\"targetIdentityCode\": \"7F3A\""));
        assert!(json.contains("\"requiresConfirmation\": true"));
    }

    fn image(status: ImagePlanStatus) -> ImagePlan {
        ImagePlan {
            source: "C:\\images\\sample.iso".into(),
            file_name: "sample.iso".to_string(),
            size_bytes: 4096,
            status,
            status_reason: status.as_str().to_string(),
            kind: ImageKind::Iso,
            detection_method: DetectionMethod::Signature,
            signatures: Vec::new(),
            hybrid: false,
            boot_metadata: None,
            write_mode: WriteMode::FileSystem,
            write_strategy: match status {
                ImagePlanStatus::Ready => ImageWriteStrategy::IsoFileSystemBootSetup,
                ImagePlanStatus::NeedsInspection => ImageWriteStrategy::IsoNeedsDeeperInspection,
                ImagePlanStatus::Blocked => ImageWriteStrategy::Unknown,
            },
            iso_files: None,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn target(blocked: bool) -> TargetAssessment {
        TargetAssessment {
            index: 2,
            path: "\\\\.\\PhysicalDrive2".to_string(),
            status: if blocked {
                TargetAssessmentStatus::Blocked
            } else {
                TargetAssessmentStatus::NeedsConfirmation
            },
            reason: if blocked {
                "target contains the system volume"
            } else {
                "target looks eligible but still requires explicit confirmation"
            }
            .to_string(),
            size_bytes: Some(32 * 1024 * 1024),
            model: Some("Test Drive".to_string()),
            interface_type: Some("USB".to_string()),
            media_type: Some("Removable Media".to_string()),
            serial_number: Some("SN-0001".to_string()),
            identity_code: Some("7F3A".to_string()),
            access: DeviceAccess::Readable,
            contains_system_volume: blocked,
            likely_removable: !blocked,
            requires_confirmation: !blocked,
            contents: None,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
