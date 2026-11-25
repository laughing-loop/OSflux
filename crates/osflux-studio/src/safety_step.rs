//! Step 3 (frames 07–08): the engine's safety check verdict, its checks in
//! order, and the technical details in the engine's own values. Studio adds
//! one row of its own, Verification, from the setting.

use osflux_engine::{
    DriveContentsState, ImageFamily, ImageWriteStrategy, IsoFilesystemStrategy, SafetyCheck,
    SafetyCheckItem, SafetyCheckKind, SafetyCheckState, WritePreflightStatus,
};

use crate::drive_step::blocked_reason;
use crate::format::{bytes_exact, size_short};
use crate::image_step::{boot_catalog_valid, boot_file_paths, family_label};
use crate::steps::Flow;
use crate::{DriveKind, Status};

/// What the button next to a blocked verdict does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    ChooseDrive,
    ChooseImage,
}

impl Fix {
    pub fn label(self) -> &'static str {
        match self {
            Self::ChooseDrive => "Choose another drive",
            Self::ChooseImage => "Choose another image",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckRow {
    pub status: Status,
    pub title: &'static str,
    pub detail: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SafetyView {
    pub status: Status,
    pub title: &'static str,
    pub description: String,
    /// "5 of 5 checks passed", when there is nothing to fix.
    pub count: String,
    pub fix: Option<Fix>,
    pub rows: Vec<CheckRow>,
    pub details_left: Vec<Detail>,
    pub details_right: Vec<Detail>,
    /// Long values that take the panel's whole width, under the columns.
    pub details_wide: Vec<Detail>,
    pub rail_subtitle: String,
    pub rail_blocked: bool,
    pub hint: &'static str,
    pub can_continue: bool,
}

/// What Studio knows about the image and drive beyond the safety check.
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub flow: Flow,
    pub verify: bool,
    pub family: ImageFamily,
    pub filesystem_strategy: Option<IsoFilesystemStrategy>,
    pub drive_kind: DriveKind,
}

pub fn safety_view(check: &SafetyCheck, context: Context) -> SafetyView {
    let first_failed = check
        .checks
        .iter()
        .find(|item| item.state == SafetyCheckState::Failed);
    let stopped = first_failed.is_some();

    let mut rows: Vec<CheckRow> = Vec::with_capacity(5);
    let mut not_checked_shown = false;
    for item in &check.checks {
        rows.push(check_row(check, item, context, &mut not_checked_shown));
    }
    rows.push(verification_row(
        stopped,
        context.verify,
        &mut not_checked_shown,
    ));
    let passed = check.passed_count() + usize::from(!stopped && context.verify);

    let (status, title) = match check.preflight.status {
        WritePreflightStatus::Ready => (Status::Ready, "Ready"),
        WritePreflightStatus::NeedsConfirmation => {
            (Status::NeedsConfirmation, "Needs confirmation")
        }
        WritePreflightStatus::NeedsInspection => (Status::NeedsInspection, "Needs inspection"),
        WritePreflightStatus::Blocked => (Status::Blocked, "Blocked"),
    };

    let unknown_contents = check
        .target
        .contents
        .as_ref()
        .is_none_or(|contents| contents.state == DriveContentsState::Unknown);

    let (description, fix, rail_subtitle, hint) = match check.preflight.status {
        WritePreflightStatus::Ready => (
            if context.flow == Flow::Windows {
                "Everything checks out. Next, choose Windows options, then confirm the drive."
                    .to_string()
            } else {
                "Everything checks out. Next, confirm the drive.".to_string()
            },
            None,
            "Verdict: Ready".to_string(),
            "All checks passed.",
        ),
        WritePreflightStatus::NeedsConfirmation => (
            if unknown_contents {
                "Everything checks out. OSflux couldn't check what's on this drive, so it will ask for the full phrase before erasing it."
                    .to_string()
            } else {
                "Everything checks out. This drive has files on it, so OSflux will ask for the full phrase before erasing it."
                    .to_string()
            },
            None,
            "Verdict: Needs confirmation".to_string(),
            "Checks passed. This drive needs the full phrase.",
        ),
        WritePreflightStatus::NeedsInspection => (
            "The engine can't trust this image's boot information yet, so it can't be written. Choose another image."
                .to_string(),
            Some(Fix::ChooseImage),
            "Verdict: Needs inspection".to_string(),
            "This image can't be written yet.",
        ),
        WritePreflightStatus::Blocked => {
            let (description, fix, short) = blocked_explanation(check, first_failed);
            (
                description,
                Some(fix),
                format!("Blocked: {short}"),
                "Fix the failed check to continue.",
            )
        }
    };

    let can_continue = matches!(
        check.preflight.status,
        WritePreflightStatus::Ready | WritePreflightStatus::NeedsConfirmation
    );
    let (details_left, details_right) = details(check, context, &rows);
    let boot_files = boot_file_paths(&check.image);
    let details_wide = if boot_files.is_empty() {
        Vec::new()
    } else {
        vec![("boot.files".to_string(), boot_files.join(" · "))]
    };

    SafetyView {
        status,
        title,
        description,
        count: if fix.is_none() {
            format!("{passed} of {} checks passed", rows.len())
        } else {
            String::new()
        },
        fix,
        rows,
        details_left,
        details_right,
        details_wide,
        rail_subtitle,
        rail_blocked: check.preflight.status == WritePreflightStatus::Blocked,
        hint,
        can_continue,
    }
}

fn check_title(kind: SafetyCheckKind) -> &'static str {
    match kind {
        SafetyCheckKind::ImagePlanTrusted => "Image plan trusted",
        SafetyCheckKind::TargetAllowed => "Target drive allowed",
        SafetyCheckKind::ImageFits => "Image fits on drive",
        SafetyCheckKind::WriteMethodSelected => "Write method selected",
    }
}

fn state_status(state: SafetyCheckState) -> Status {
    match state {
        SafetyCheckState::Passed => Status::Ready,
        SafetyCheckState::NeedsConfirmation => Status::NeedsConfirmation,
        SafetyCheckState::NeedsInspection => Status::NeedsInspection,
        SafetyCheckState::Failed => Status::Blocked,
        SafetyCheckState::NotChecked => Status::NotChecked,
    }
}

fn not_checked(shown: &mut bool) -> String {
    if *shown {
        "Not checked.".to_string()
    } else {
        *shown = true;
        "Not checked. The engine stops at the first failure.".to_string()
    }
}

fn check_row(
    check: &SafetyCheck,
    item: &SafetyCheckItem,
    context: Context,
    not_checked_shown: &mut bool,
) -> CheckRow {
    let status = state_status(item.state);
    let title = check_title(item.kind);
    if item.state == SafetyCheckState::NotChecked {
        return CheckRow {
            status,
            title,
            detail: not_checked(not_checked_shown),
            value: "–".to_string(),
        };
    }
    let image = &check.image;
    let target = &check.target;
    let (detail, value) = match item.kind {
        SafetyCheckKind::ImagePlanTrusted => {
            let detail = match item.state {
                SafetyCheckState::Passed => {
                    let family = family_label(context.family);
                    if boot_catalog_valid(image) {
                        format!("{family}. Boot catalog valid.")
                    } else {
                        format!("{family}.")
                    }
                }
                SafetyCheckState::NeedsInspection => {
                    "Boot information needs a closer look.".to_string()
                }
                _ => "The engine blocked this image.".to_string(),
            };
            (detail, String::new())
        }
        SafetyCheckKind::TargetAllowed => {
            let detail = if check.target_changed {
                "The drive changed after you chose it.".to_string()
            } else {
                match item.state {
                    SafetyCheckState::Passed => match context.drive_kind {
                        DriveKind::SdCard => "SD card, not a system drive.".to_string(),
                        DriveKind::Usb | DriveKind::Disk => {
                            "Removable USB stick, not a system drive.".to_string()
                        }
                    },
                    SafetyCheckState::NeedsConfirmation => {
                        let unknown = target
                            .contents
                            .as_ref()
                            .is_none_or(|contents| contents.state == DriveContentsState::Unknown);
                        if unknown {
                            "Allowed. OSflux couldn't check what's on it.".to_string()
                        } else {
                            "Allowed. It has files on it, which will be erased.".to_string()
                        }
                    }
                    _ => format!("{}.", blocked_reason(&target.reason)),
                }
            };
            (detail, target.path.clone())
        }
        SafetyCheckKind::ImageFits => match target.size_bytes {
            Some(size) => {
                let value = format!("{} of {}", size_short(image.size_bytes), size_short(size));
                let detail = if item.state == SafetyCheckState::Failed {
                    "Image is larger than the drive."
                } else if size - image.size_bytes >= size / 10 {
                    "Plenty of room left over."
                } else {
                    "Fits, with little room to spare."
                };
                (detail.to_string(), value)
            }
            None => (
                "The drive's size could not be read.".to_string(),
                "–".to_string(),
            ),
        },
        SafetyCheckKind::WriteMethodSelected => {
            let detail = match (item.state, image.write_strategy) {
                (SafetyCheckState::Passed, ImageWriteStrategy::IsoFileSystemBootSetup) => {
                    "Files copied to a FAT32 stick (UEFI).".to_string()
                }
                (SafetyCheckState::Passed, ImageWriteStrategy::HybridIsoRawWrite) => {
                    "Hybrid ISO copied byte for byte.".to_string()
                }
                (SafetyCheckState::Passed, _) => "Disk image copied byte for byte.".to_string(),
                (SafetyCheckState::NeedsInspection, _) => "Needs deeper inspection.".to_string(),
                _ => "The engine has no way to write this image.".to_string(),
            };
            (detail, String::new())
        }
    };
    CheckRow {
        status,
        title,
        detail,
        value,
    }
}

fn verification_row(stopped: bool, verify: bool, not_checked_shown: &mut bool) -> CheckRow {
    let title = "Verification";
    if stopped {
        CheckRow {
            status: Status::NotChecked,
            title,
            detail: not_checked(not_checked_shown),
            value: "–".to_string(),
        }
    } else if verify {
        CheckRow {
            status: Status::Ready,
            title,
            detail: "Every byte is read back and compared.".to_string(),
            value: "on".to_string(),
        }
    } else {
        CheckRow {
            status: Status::NeedsInspection,
            title,
            detail: "Off. A fake or failing stick won't be caught.".to_string(),
            value: "off".to_string(),
        }
    }
}

/// The description, the fix and the rail's short reason for a blocked
/// verdict, from the first check that failed.
fn blocked_explanation(
    check: &SafetyCheck,
    first_failed: Option<&SafetyCheckItem>,
) -> (String, Fix, &'static str) {
    if check.target_changed {
        return (
            "The drive changed after you chose it: it was unplugged, or another drive took its place. Choose the drive again."
                .to_string(),
            Fix::ChooseDrive,
            "drive changed",
        );
    }
    let Some(item) = first_failed else {
        return (
            format!("The engine blocked this write: {}.", check.preflight.reason),
            Fix::ChooseDrive,
            "see the checks",
        );
    };
    match item.kind {
        SafetyCheckKind::ImagePlanTrusted => (
            "The engine blocked this image. Choose another image.".to_string(),
            Fix::ChooseImage,
            "image not trusted",
        ),
        SafetyCheckKind::TargetAllowed => (
            format!(
                "OSflux will not erase this drive: {}. Choose another drive.",
                blocked_reason(&check.target.reason)
            ),
            Fix::ChooseDrive,
            "drive not allowed",
        ),
        SafetyCheckKind::ImageFits => match check.target.size_bytes {
            Some(size) => (
                format!(
                    "Image is larger than the drive. It needs {}, but this drive holds {}. Choose a drive of {} or more.",
                    size_short(check.image.size_bytes),
                    size_short(size),
                    suggested_drive_size(check.image.size_bytes)
                ),
                Fix::ChooseDrive,
                "image too large",
            ),
            None => (
                "OSflux couldn't read this drive's size, so it can't tell whether the image fits. Choose another drive."
                    .to_string(),
                Fix::ChooseDrive,
                "drive size unknown",
            ),
        },
        SafetyCheckKind::WriteMethodSelected => (
            "The engine has no way to write this image. Choose another image.".to_string(),
            Fix::ChooseImage,
            "no write method",
        ),
    }
}

/// Why the safety check could not run at all.
#[derive(Debug, Clone, PartialEq)]
pub enum SafetyFailure {
    /// No drive is at the chosen position any more.
    DriveGone,
    /// The image could not be read again.
    Image(String),
}

/// The verdict screen when the check could not run: blocked, with the way
/// out.
pub fn failure_view(failure: &SafetyFailure) -> SafetyView {
    let (description, fix, short) = match failure {
        SafetyFailure::DriveGone => (
            "The drive is no longer connected. Plug it back in and choose it again, or choose another drive."
                .to_string(),
            Fix::ChooseDrive,
            "drive not found",
        ),
        SafetyFailure::Image(error) => (
            format!("OSflux couldn't read the image again: {error}. Choose the image again."),
            Fix::ChooseImage,
            "image not readable",
        ),
    };
    SafetyView {
        status: Status::Blocked,
        title: "Blocked",
        description,
        count: String::new(),
        fix: Some(fix),
        rows: Vec::new(),
        details_left: Vec::new(),
        details_right: Vec::new(),
        details_wide: Vec::new(),
        rail_subtitle: format!("Blocked: {short}"),
        rail_blocked: true,
        hint: "Fix the failed check to continue.",
        can_continue: false,
    }
}

/// The smallest drive size sold that holds the image. Drives are sold in
/// decimal gigabytes and hold a little less than their label, so a "16 GB"
/// stick counts as 90% of 16,000,000,000 bytes.
pub fn suggested_drive_size(image_bytes: u64) -> String {
    const LABELS_GB: [u64; 13] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1000, 2000, 4000];
    for gb in LABELS_GB {
        if gb * 900_000_000 >= image_bytes {
            return if gb >= 1000 {
                format!("{} TB", gb / 1000)
            } else {
                format!("{gb} GB")
            };
        }
    }
    size_short(image_bytes)
}

/// An engine key and its value, for technical details.
pub type Detail = (String, String);

fn details(check: &SafetyCheck, context: Context, rows: &[CheckRow]) -> (Vec<Detail>, Vec<Detail>) {
    let image = &check.image;
    let target = &check.target;
    let method_state = rows
        .iter()
        .find(|row| row.title == check_title(SafetyCheckKind::WriteMethodSelected))
        .map(|row| match row.status {
            Status::Ready => "selected",
            Status::NeedsInspection => "needs-inspection",
            Status::NotChecked => "not-checked",
            _ => "blocked",
        })
        .unwrap_or("blocked");
    let method = match (image.write_strategy, context.filesystem_strategy) {
        (ImageWriteStrategy::IsoFileSystemBootSetup, Some(strategy)) => strategy.as_str(),
        (ImageWriteStrategy::IsoFileSystemBootSetup, None) => image.write_strategy.as_str(),
        (ImageWriteStrategy::HybridIsoRawWrite | ImageWriteStrategy::RawBlockWrite, _) => {
            "raw-block-write"
        }
        (strategy, _) => strategy.as_str(),
    };

    let left = vec![
        ("image.size".to_string(), bytes_exact(image.size_bytes)),
        (
            "target.size".to_string(),
            target
                .size_bytes
                .map(bytes_exact)
                .unwrap_or_else(|| "unknown".to_string()),
        ),
        (
            "write.mode".to_string(),
            format!("{method_state} · {method}"),
        ),
    ];

    let mut target_parts = vec![target.path.clone()];
    if let Some(bus) = target
        .interface_type
        .as_deref()
        .filter(|bus| !bus.is_empty())
    {
        target_parts.push(bus.to_ascii_lowercase());
    }
    if let Some(media) = target
        .media_type
        .as_deref()
        .filter(|media| !media.is_empty())
    {
        target_parts.push(media.to_ascii_lowercase());
    }
    let right = vec![
        ("target".to_string(), target_parts.join(" · ")),
        (
            "target.status".to_string(),
            target.status.as_str().to_string(),
        ),
        (
            "verify".to_string(),
            if context.verify {
                "read-back on · every byte".to_string()
            } else {
                "read-back off".to_string()
            },
        ),
    ];
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::{safety_view, suggested_drive_size, Context, Fix};
    use crate::drive_step::tests::assessment;
    use crate::steps::Flow;
    use crate::{DriveKind, Status};
    use osflux_engine::{
        build_safety_check, DetectionMethod, ImageFamily, ImageKind, ImagePlan, ImagePlanStatus,
        ImageSignature, ImageWriteStrategy, IsoFilesystemStrategy, TargetAssessmentStatus,
        WriteMode,
    };
    use std::path::PathBuf;

    fn windows_plan(size: u64) -> ImagePlan {
        ImagePlan {
            source: PathBuf::from("D:/Images/Windows11.iso"),
            file_name: "Windows11.iso".into(),
            size_bytes: size,
            status: ImagePlanStatus::Ready,
            status_reason: "image plan is trusted for the next safety phase".into(),
            kind: ImageKind::Iso,
            detection_method: DetectionMethod::Signature,
            signatures: vec![ImageSignature::Iso9660, ImageSignature::Udf],
            hybrid: false,
            boot_metadata: None,
            write_mode: WriteMode::FileSystem,
            write_strategy: ImageWriteStrategy::IsoFileSystemBootSetup,
            iso_files: None,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn context(verify: bool) -> Context {
        Context {
            flow: Flow::Windows,
            verify,
            family: ImageFamily::WindowsInstaller,
            filesystem_strategy: Some(IsoFilesystemStrategy::WindowsInstallerBootSetup),
            drive_kind: DriveKind::Usb,
        }
    }

    #[test]
    fn an_empty_stick_that_fits_matches_frame_07() {
        let target = assessment(
            2,
            "SanDisk Ultra 64GB",
            64_023_511_040,
            TargetAssessmentStatus::Eligible,
            "target is empty, so its identity code is enough to confirm a write",
        );
        let check = build_safety_check(windows_plan(9_030_170_624), target, false);
        let view = safety_view(&check, context(true));

        assert_eq!(view.status, Status::Ready);
        assert_eq!(view.title, "Ready");
        assert_eq!(
            view.description,
            "Everything checks out. Next, choose Windows options, then confirm the drive."
        );
        assert_eq!(view.count, "5 of 5 checks passed");
        assert_eq!(view.fix, None);
        assert!(view.can_continue);
        let rows: Vec<_> = view
            .rows
            .iter()
            .map(|row| (row.status, row.title, row.value.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (Status::Ready, "Image plan trusted", ""),
                (
                    Status::Ready,
                    "Target drive allowed",
                    "\\\\.\\PhysicalDrive2"
                ),
                (Status::Ready, "Image fits on drive", "8.41 GB of 59.6 GB"),
                (Status::Ready, "Write method selected", ""),
                (Status::Ready, "Verification", "on"),
            ]
        );
        assert_eq!(view.rows[2].detail, "Plenty of room left over.");
        assert_eq!(view.rows[3].detail, "Files copied to a FAT32 stick (UEFI).");
        assert_eq!(view.rail_subtitle, "Verdict: Ready");
        assert!(view.details_left.contains(&(
            "write.mode".to_string(),
            "selected · windows-installer-boot-setup".to_string()
        )));
        assert!(view.details_right.contains(&(
            "target".to_string(),
            "\\\\.\\PhysicalDrive2 · usb · removable media".to_string()
        )));
        assert!(view.details_left.contains(&(
            "target.size".to_string(),
            "64,023,511,040 B (59.6 GiB)".to_string()
        )));
    }

    #[test]
    fn a_stick_that_is_too_small_matches_frame_08() {
        let target = assessment(
            3,
            "Kingston DataTraveler 8GB",
            8_004_304_896,
            TargetAssessmentStatus::Eligible,
            "target is empty",
        );
        let check = build_safety_check(windows_plan(9_030_170_624), target, false);
        let view = safety_view(&check, context(true));

        assert_eq!(view.status, Status::Blocked);
        assert_eq!(
            view.description,
            "Image is larger than the drive. It needs 8.41 GB, but this drive holds 7.45 GB. Choose a drive of 16 GB or more."
        );
        assert_eq!(view.fix, Some(Fix::ChooseDrive));
        assert_eq!(view.count, "");
        assert_eq!(view.rail_subtitle, "Blocked: image too large");
        assert!(view.rail_blocked);
        assert!(!view.can_continue);
        assert_eq!(view.hint, "Fix the failed check to continue.");
        let statuses: Vec<_> = view.rows.iter().map(|row| row.status).collect();
        assert_eq!(
            statuses,
            [
                Status::Ready,
                Status::Ready,
                Status::Blocked,
                Status::NotChecked,
                Status::NotChecked
            ]
        );
        assert_eq!(view.rows[2].value, "8.41 GB of 7.45 GB");
        assert_eq!(
            view.rows[3].detail,
            "Not checked. The engine stops at the first failure."
        );
        assert_eq!(view.rows[4].detail, "Not checked.");
        assert_eq!(view.rows[4].value, "–");
    }

    #[test]
    fn the_full_list_of_boot_files_is_in_the_technical_details() {
        use osflux_engine::{
            IsoBootAsset, IsoBootAssetKind, IsoBootMetadata, IsoDirectoryMetadata,
        };
        let mut plan = windows_plan(1 << 30);
        let paths = [
            "BOOTMGR",
            "SOURCES/INSTALL.WIM",
            "EFI/BOOT/BOOTX64.EFI",
            "EFI/MICROSOFT/BOOT",
            "EFI/MICROSOFT/BOOT/EFISYS.BIN",
        ];
        plan.boot_metadata = Some(IsoBootMetadata {
            el_torito: None,
            directory: Some(IsoDirectoryMetadata {
                root_extent_lba: 18,
                root_data_length: 2048,
                scanned_directories: 3,
                entries: Vec::new(),
                boot_assets: paths
                    .iter()
                    .map(|path| IsoBootAsset {
                        kind: IsoBootAssetKind::UefiBootloader,
                        path: path.to_string(),
                    })
                    .collect(),
                truncated: false,
            }),
        });
        let target = assessment(
            2,
            "SanDisk",
            64_023_511_040,
            TargetAssessmentStatus::Eligible,
            "",
        );
        let view = safety_view(&build_safety_check(plan, target, false), context(true));
        assert_eq!(
            view.details_wide,
            [(
                "boot.files".to_string(),
                "/bootmgr · /sources/install.wim · /efi/boot/bootx64.efi · /efi/microsoft/boot/efisys.bin"
                    .to_string()
            )]
        );
    }

    #[test]
    fn verification_off_is_amber_and_not_counted() {
        let target = assessment(
            2,
            "SanDisk",
            64_023_511_040,
            TargetAssessmentStatus::Eligible,
            "",
        );
        let check = build_safety_check(windows_plan(1 << 30), target, false);
        let view = safety_view(&check, context(false));
        assert_eq!(view.rows[4].status, Status::NeedsInspection);
        assert_eq!(view.rows[4].value, "off");
        assert_eq!(view.count, "4 of 5 checks passed");
        assert!(view.can_continue);
    }

    #[test]
    fn a_changed_drive_is_blocked_with_choose_again() {
        let target = assessment(
            2,
            "SanDisk",
            64_023_511_040,
            TargetAssessmentStatus::Eligible,
            "",
        );
        let check = build_safety_check(windows_plan(1 << 30), target, true);
        let view = safety_view(&check, context(true));
        assert_eq!(view.status, Status::Blocked);
        assert_eq!(view.rail_subtitle, "Blocked: drive changed");
        assert_eq!(view.rows[1].detail, "The drive changed after you chose it.");
        assert_eq!(view.fix, Some(Fix::ChooseDrive));
    }

    #[test]
    fn a_drive_with_files_needs_confirmation() {
        let target = assessment(
            4,
            "SD card",
            15_931_539_456,
            TargetAssessmentStatus::NeedsConfirmation,
            "target looks eligible but still requires explicit confirmation",
        );
        let check = build_safety_check(windows_plan(1 << 30), target, false);
        let mut context = context(true);
        context.flow = Flow::Other;
        let view = safety_view(&check, context);
        assert_eq!(view.status, Status::NeedsConfirmation);
        assert_eq!(view.rows[1].status, Status::NeedsConfirmation);
        assert_eq!(view.count, "5 of 5 checks passed");
        assert!(view.can_continue);
        assert!(view
            .description
            .contains("couldn't check what's on this drive"));
    }

    #[test]
    fn suggested_sizes_leave_room_for_what_drives_really_hold() {
        assert_eq!(suggested_drive_size(9_030_170_624), "16 GB");
        assert_eq!(suggested_drive_size(2_832_413_696), "4 GB");
        assert_eq!(suggested_drive_size(7_300_000_000), "16 GB");
        assert_eq!(suggested_drive_size(600_000_000_000), "1 TB");
    }
}
