//! The safety check Studio shows before a drive is confirmed: the engine's
//! preflight verdict, broken into the checks it is made of, in order.
//!
//! The checks follow the same rules as [`build_write_preflight`], so the
//! list and the verdict always agree. Like the preflight, it reads the image
//! and the drive again rather than trusting what an earlier screen showed,
//! and it notices when a different drive has taken the chosen drive's place.

use std::fmt;
use std::path::Path;

use crate::image_plan::{
    plan_image, ImagePlan, ImagePlanError, ImagePlanStatus, ImageWriteStrategy,
};
use crate::preflight::{build_write_preflight, WritePreflight, WritePreflightStatus};
use crate::target::{
    assess_target, TargetAssessment, TargetAssessmentError, TargetAssessmentStatus,
};

/// One of the engine's checks, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyCheckKind {
    /// The image plan is `ready`.
    ImagePlanTrusted,
    /// The drive is one OSflux may write to.
    TargetAllowed,
    /// The image is no larger than the drive.
    ImageFits,
    /// The engine has a write method for this image.
    WriteMethodSelected,
}

impl SafetyCheckKind {
    pub const ALL: [SafetyCheckKind; 4] = [
        Self::ImagePlanTrusted,
        Self::TargetAllowed,
        Self::ImageFits,
        Self::WriteMethodSelected,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImagePlanTrusted => "image-plan-trusted",
            Self::TargetAllowed => "target-allowed",
            Self::ImageFits => "image-fits",
            Self::WriteMethodSelected => "write-method-selected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyCheckState {
    Passed,
    /// Passed, but the drive holds data, so the full phrase is needed.
    NeedsConfirmation,
    NeedsInspection,
    Failed,
    /// Not reached: the checks stop at the first one that fails.
    NotChecked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyCheckItem {
    pub kind: SafetyCheckKind,
    pub state: SafetyCheckState,
    /// The engine's reason, in its own words.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyCheck {
    /// The engine's verdict, as `--preflight` reports it.
    pub preflight: WritePreflight,
    pub checks: Vec<SafetyCheckItem>,
    pub image: ImagePlan,
    pub target: TargetAssessment,
    /// The chosen drive is no longer the drive at this position: unplugged,
    /// or replaced by another. The verdict is then `blocked`.
    pub target_changed: bool,
}

impl SafetyCheck {
    /// How many checks passed, including one that needs the full phrase.
    pub fn passed_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| {
                matches!(
                    check.state,
                    SafetyCheckState::Passed | SafetyCheckState::NeedsConfirmation
                )
            })
            .count()
    }
}

#[derive(Debug)]
pub enum SafetyCheckError {
    Image(ImagePlanError),
    Target(TargetAssessmentError),
}

impl fmt::Display for SafetyCheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => write!(formatter, "image planning failed: {error}"),
            Self::Target(error) => write!(formatter, "target check failed: {error}"),
        }
    }
}

impl std::error::Error for SafetyCheckError {}

/// Plans the image and assesses the drive at `target_index` afresh, then
/// runs the checks. `expected_identity` is the identity code of the drive
/// that was chosen; when another drive now has that index, or the drive is
/// gone, the check is blocked with `target_changed` set.
pub fn run_safety_check(
    image_path: impl AsRef<Path>,
    target_index: u32,
    expected_identity: Option<&str>,
) -> Result<SafetyCheck, SafetyCheckError> {
    let image = plan_image(image_path).map_err(SafetyCheckError::Image)?;
    let target = assess_target(target_index).map_err(SafetyCheckError::Target)?;
    let target_changed =
        expected_identity.is_some_and(|expected| target.identity_code.as_deref() != Some(expected));
    Ok(build_safety_check(image, target, target_changed))
}

pub fn build_safety_check(
    image: ImagePlan,
    target: TargetAssessment,
    target_changed: bool,
) -> SafetyCheck {
    let mut preflight = build_write_preflight(&image, &target);
    if target_changed {
        preflight.status = WritePreflightStatus::Blocked;
        preflight.reason = "target changed since it was chosen".to_string();
    }

    let mut checks = Vec::with_capacity(SafetyCheckKind::ALL.len());
    let mut stopped = false;
    for kind in SafetyCheckKind::ALL {
        if stopped {
            checks.push(SafetyCheckItem {
                kind,
                state: SafetyCheckState::NotChecked,
                reason: "not checked: an earlier check failed".to_string(),
            });
            continue;
        }
        let (state, reason) = evaluate(kind, &image, &target, target_changed);
        // A failed check blocks the write, so the rest are not reached. An
        // image that needs inspection does not stop them: a drive that is
        // blocked or too small still has to be reported, as the preflight
        // reports it.
        stopped = state == SafetyCheckState::Failed;
        checks.push(SafetyCheckItem {
            kind,
            state,
            reason,
        });
    }

    SafetyCheck {
        preflight,
        checks,
        image,
        target,
        target_changed,
    }
}

fn evaluate(
    kind: SafetyCheckKind,
    image: &ImagePlan,
    target: &TargetAssessment,
    target_changed: bool,
) -> (SafetyCheckState, String) {
    match kind {
        SafetyCheckKind::ImagePlanTrusted => match image.status {
            ImagePlanStatus::Ready => (SafetyCheckState::Passed, image.status_reason.clone()),
            ImagePlanStatus::NeedsInspection => (
                SafetyCheckState::NeedsInspection,
                image.status_reason.clone(),
            ),
            ImagePlanStatus::Blocked => (SafetyCheckState::Failed, image.status_reason.clone()),
        },
        SafetyCheckKind::TargetAllowed => {
            if target_changed {
                return (
                    SafetyCheckState::Failed,
                    "target changed since it was chosen".to_string(),
                );
            }
            match target.status {
                TargetAssessmentStatus::Eligible => {
                    (SafetyCheckState::Passed, target.reason.clone())
                }
                TargetAssessmentStatus::NeedsConfirmation => {
                    (SafetyCheckState::NeedsConfirmation, target.reason.clone())
                }
                TargetAssessmentStatus::Blocked => {
                    (SafetyCheckState::Failed, target.reason.clone())
                }
            }
        }
        SafetyCheckKind::ImageFits => match target.size_bytes {
            Some(size) if image.size_bytes > size => (
                SafetyCheckState::Failed,
                "image is larger than the target device".to_string(),
            ),
            Some(_) => (
                SafetyCheckState::Passed,
                "image fits on the target device".to_string(),
            ),
            None => (
                SafetyCheckState::Failed,
                "target size is unknown".to_string(),
            ),
        },
        SafetyCheckKind::WriteMethodSelected => match image.write_strategy {
            ImageWriteStrategy::IsoFileSystemBootSetup
            | ImageWriteStrategy::HybridIsoRawWrite
            | ImageWriteStrategy::RawBlockWrite => (
                SafetyCheckState::Passed,
                image.write_strategy.as_str().to_string(),
            ),
            ImageWriteStrategy::IsoNeedsDeeperInspection => (
                SafetyCheckState::NeedsInspection,
                image.write_strategy.as_str().to_string(),
            ),
            ImageWriteStrategy::Unknown => (
                SafetyCheckState::Failed,
                image.write_strategy.as_str().to_string(),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{build_safety_check, SafetyCheckKind, SafetyCheckState};
    use crate::image_plan::{
        DetectionMethod, ImageKind, ImagePlan, ImagePlanStatus, ImageWriteStrategy, WriteMode,
    };
    use crate::preflight::WritePreflightStatus;
    use crate::target::{TargetAssessment, TargetAssessmentStatus};
    use osflux_core::device::DeviceAccess;

    const MIB: u64 = 1024 * 1024;

    fn states(check: &super::SafetyCheck) -> Vec<SafetyCheckState> {
        check.checks.iter().map(|item| item.state).collect()
    }

    #[test]
    fn a_ready_image_on_an_empty_drive_passes_every_check() {
        let check = build_safety_check(
            image(ImagePlanStatus::Ready, 8 * MIB),
            target(TargetAssessmentStatus::Eligible, 32 * MIB),
            false,
        );
        assert_eq!(check.preflight.status, WritePreflightStatus::Ready);
        assert_eq!(states(&check), [SafetyCheckState::Passed; 4]);
        assert_eq!(check.passed_count(), 4);
        let kinds: Vec<_> = check.checks.iter().map(|item| item.kind).collect();
        assert_eq!(kinds, SafetyCheckKind::ALL);
    }

    #[test]
    fn a_drive_with_files_needs_confirmation_but_the_checks_go_on() {
        let check = build_safety_check(
            image(ImagePlanStatus::Ready, 8 * MIB),
            target(TargetAssessmentStatus::NeedsConfirmation, 32 * MIB),
            false,
        );
        assert_eq!(
            check.preflight.status,
            WritePreflightStatus::NeedsConfirmation
        );
        assert_eq!(
            states(&check),
            [
                SafetyCheckState::Passed,
                SafetyCheckState::NeedsConfirmation,
                SafetyCheckState::Passed,
                SafetyCheckState::Passed
            ]
        );
        assert_eq!(check.passed_count(), 4);
    }

    #[test]
    fn an_image_larger_than_the_drive_stops_at_the_size_check() {
        // Frame 08: 8.41 GB image, 7.45 GB stick.
        let check = build_safety_check(
            image(ImagePlanStatus::Ready, 64 * MIB),
            target(TargetAssessmentStatus::Eligible, 32 * MIB),
            false,
        );
        assert_eq!(check.preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(
            check.preflight.reason,
            "image is larger than the target device"
        );
        assert_eq!(
            states(&check),
            [
                SafetyCheckState::Passed,
                SafetyCheckState::Passed,
                SafetyCheckState::Failed,
                SafetyCheckState::NotChecked
            ]
        );
        assert_eq!(check.passed_count(), 2);
    }

    #[test]
    fn a_blocked_drive_stops_the_checks() {
        let check = build_safety_check(
            image(ImagePlanStatus::Ready, 8 * MIB),
            target(TargetAssessmentStatus::Blocked, 32 * MIB),
            false,
        );
        assert_eq!(check.preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(check.checks[1].state, SafetyCheckState::Failed);
        assert_eq!(check.checks[1].reason, "target contains the system volume");
        assert_eq!(check.checks[2].state, SafetyCheckState::NotChecked);
        assert_eq!(check.checks[3].state, SafetyCheckState::NotChecked);
    }

    #[test]
    fn an_image_needing_inspection_is_reported_with_the_drive_checks() {
        let check = build_safety_check(
            image(ImagePlanStatus::NeedsInspection, 8 * MIB),
            target(TargetAssessmentStatus::Eligible, 32 * MIB),
            false,
        );
        assert_eq!(
            check.preflight.status,
            WritePreflightStatus::NeedsInspection
        );
        assert_eq!(
            states(&check),
            [
                SafetyCheckState::NeedsInspection,
                SafetyCheckState::Passed,
                SafetyCheckState::Passed,
                SafetyCheckState::NeedsInspection
            ]
        );

        // Too large as well: the size check fails, as the preflight says.
        let check = build_safety_check(
            image(ImagePlanStatus::NeedsInspection, 64 * MIB),
            target(TargetAssessmentStatus::Eligible, 32 * MIB),
            false,
        );
        assert_eq!(check.preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(check.checks[2].state, SafetyCheckState::Failed);
        assert_eq!(check.checks[3].state, SafetyCheckState::NotChecked);
    }

    #[test]
    fn a_different_drive_at_the_same_position_blocks_the_write() {
        let check = build_safety_check(
            image(ImagePlanStatus::Ready, 8 * MIB),
            target(TargetAssessmentStatus::Eligible, 32 * MIB),
            true,
        );
        assert!(check.target_changed);
        assert_eq!(check.preflight.status, WritePreflightStatus::Blocked);
        assert_eq!(check.preflight.reason, "target changed since it was chosen");
        assert_eq!(check.checks[1].state, SafetyCheckState::Failed);
        assert_eq!(check.checks[2].state, SafetyCheckState::NotChecked);
    }

    #[test]
    fn the_checks_agree_with_the_preflight_verdict() {
        for image_status in [
            ImagePlanStatus::Ready,
            ImagePlanStatus::NeedsInspection,
            ImagePlanStatus::Blocked,
        ] {
            for target_status in [
                TargetAssessmentStatus::Eligible,
                TargetAssessmentStatus::NeedsConfirmation,
                TargetAssessmentStatus::Blocked,
            ] {
                for image_size in [8 * MIB, 64 * MIB] {
                    let check = build_safety_check(
                        image(image_status, image_size),
                        target(target_status, 32 * MIB),
                        false,
                    );
                    let worst_blocks = check
                        .checks
                        .iter()
                        .any(|item| item.state == SafetyCheckState::Failed);
                    assert_eq!(
                        worst_blocks,
                        check.preflight.status == WritePreflightStatus::Blocked,
                        "{image_status:?} {target_status:?} {image_size}"
                    );
                    let all_pass = check.checks.iter().all(|item| {
                        matches!(
                            item.state,
                            SafetyCheckState::Passed | SafetyCheckState::NeedsConfirmation
                        )
                    });
                    assert_eq!(
                        all_pass,
                        matches!(
                            check.preflight.status,
                            WritePreflightStatus::Ready | WritePreflightStatus::NeedsConfirmation
                        ),
                        "{image_status:?} {target_status:?} {image_size}"
                    );
                }
            }
        }
    }

    /// When Windows' disk details cannot be read (PowerShell missing,
    /// blocked or failing), drives are still found by opening them, with a
    /// size but no model, serial, device ID or media type, and with
    /// `contains_system_volume` left false because nothing checked it. Each
    /// such drive, the system disk included, must be refused: by its
    /// assessment, after its contents are read, and by the safety check.
    #[test]
    fn a_drive_windows_said_nothing_about_is_refused_at_every_step() {
        use crate::drive_contents::{DriveContents, DriveContentsState};
        use crate::target::{assess_target_device, with_drive_contents};
        use osflux_core::device::PhysicalDevice;

        for access in [DeviceAccess::Readable, DeviceAccess::MetadataOnly] {
            // The system disk, as listed when the PowerShell query fails.
            let system_disk = PhysicalDevice::found_without_metadata(
                0,
                r"\\.\PhysicalDrive0".to_string(),
                Some(1_000_204_886_016),
                access,
                None,
            );
            assert!(
                !system_disk.contains_system_volume,
                "the check that would say so did not run"
            );

            let assessment = assess_target_device(&system_disk);
            assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);
            assert!(!assessment.requires_confirmation);
            assert_eq!(assessment.identity_code, None);

            // Even a drive that reads as blank stays blocked.
            let blank = DriveContents {
                state: DriveContentsState::Blank,
                reason: "the first and last MiB are empty".to_string(),
                partition_scheme: None,
                partitions: Vec::new(),
            };
            let assessment = with_drive_contents(assessment, blank);
            assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);

            let check =
                build_safety_check(image(ImagePlanStatus::Ready, 8 * MIB), assessment, false);
            assert_eq!(check.preflight.status, WritePreflightStatus::Blocked);
            assert_eq!(check.checks[1].kind, SafetyCheckKind::TargetAllowed);
            assert_eq!(check.checks[1].state, SafetyCheckState::Failed);
        }
    }

    fn image(status: ImagePlanStatus, size_bytes: u64) -> ImagePlan {
        ImagePlan {
            source: "C:\\images\\sample.iso".into(),
            file_name: "sample.iso".to_string(),
            size_bytes,
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

    fn target(status: TargetAssessmentStatus, size_bytes: u64) -> TargetAssessment {
        let blocked = status == TargetAssessmentStatus::Blocked;
        TargetAssessment {
            index: 2,
            path: "\\\\.\\PhysicalDrive2".to_string(),
            status,
            reason: match status {
                TargetAssessmentStatus::Blocked => "target contains the system volume",
                TargetAssessmentStatus::Eligible => {
                    "target is empty, so its identity code is enough to confirm a write"
                }
                TargetAssessmentStatus::NeedsConfirmation => {
                    "target looks eligible but still requires explicit confirmation"
                }
            }
            .to_string(),
            size_bytes: Some(size_bytes),
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
