//! `--preview`, built only with the `preview` feature: shows a view with
//! sample data and, with `--snapshot <file.png>`, saves it as a PNG and
//! exits. Used for screenshots and design checks; it never touches a drive
//! and never saves settings.
//!
//! `--preview <view> [--image file] [--theme dark|light] [--not-admin]
//! [--verify-off] [--size WIDTHxHEIGHT] [--snapshot file.png]`, where the view is one of shell,
//! settings, about, licences, gallery, image, drives, drives-none, safety,
//! safety-blocked, the Windows options and Confirm views, writing,
//! verifying, done or done-mismatch.
//!
//! The steps read the image given with `--image` through the real engine.
//! The drives are samples, matching the design's frames, so no drive is
//! ever listed, read or written.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use osflux_engine::{
    build_safety_check, build_write_intent, build_write_preflight, plan_image, CancelToken,
    DeviceAccess, DriveContents, DriveContentsState, DrivePartition, FilesystemKind, RawWritePhase,
    RawWriteProgress, RawWriteReport, SafetyCheck, TargetAssessment, TargetAssessmentStatus,
    Verification, WriteIntentStatus,
};
use slint::ComponentHandle;

use crate::confirm_step::{ConfirmFailure, Confirmation};
use crate::drive_step::ChosenDrive;
use crate::image_step::{analyze_image, ImageAnalysis};
use crate::safety_step::SafetyFailure;
use crate::session::{Engine, Jobs};
use crate::settings::{Settings, ThemeSetting};
use crate::shell::{self, EngineSetup, Shell};
use crate::write_step::{WriteFailure, WriteRequest};
use crate::{AppWindow, Gallery, Studio, Theme};

/// How long a window is given to lay out and draw before the snapshot.
const SNAPSHOT_DELAY: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Shell,
    Settings,
    About,
    Licences,
    Gallery,
    /// Frames 02–04: the image given with `--image`.
    Image,
    /// Frame 05, with the first drive picked.
    Drives,
    /// Frame 06: only drives OSflux will not erase.
    DrivesNone,
    /// Frame 07: a 64 GB stick.
    Safety,
    /// Frame 08: an 8 GB stick.
    SafetyBlocked,
    /// Frame 09: Windows options, all off.
    WindowsOptions,
    /// Frame 10: three Windows options on, local account "alex".
    WindowsOptionsSet,
    /// Frame 11: the drive code, nothing typed yet.
    ConfirmCode,
    /// Frame 12: a wrong code.
    ConfirmCodeWrong,
    /// Frame 13: the matching code.
    ConfirmCodeMatch,
    /// Frame 14: an SD card with files, its code typed and the box naming
    /// what will be lost not yet ticked (use a Linux image and
    /// --verify-off).
    ConfirmFiles,
    /// Frame 14, with the box ticked: Write to drive unlocks.
    ConfirmFilesTicked,
    /// The full phrase, for an SD card whose contents could not be read.
    ConfirmPhrase,
    /// Frame 21: the confirmation has expired.
    ConfirmExpired,
    /// Frame 15 (17 with --verify-off): writing, 46% done.
    Writing,
    /// Frame 16: verifying, after writing.
    Verifying,
    /// Frame 18 (19 with --verify-off): written, with the expected SHA-256
    /// entered and matching.
    Done,
    /// Frame 20: written, but the expected SHA-256 differs.
    DoneMismatch,
}

/// The image's SHA-256 in the design's frames.
const SAMPLE_SHA256: &str = "2A1AB0D46D7285F738D082858925D3303F3DC430F0A4EAB5AA238325A4274906";
const SAMPLE_IMAGE_BYTES: u64 = 9_030_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub view: View,
    pub settings: Settings,
    pub administrator: bool,
    pub image: Option<PathBuf>,
    pub snapshot: Option<PathBuf>,
    /// `--size 960x640`: the window's size, to check the layout when it is
    /// small. The design's size otherwise.
    pub size: Option<(u32, u32)>,
}

impl Options {
    /// `Some` when the arguments ask for a preview.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut args = args.into_iter().peekable();
        if args.next().as_deref() != Some("--preview") {
            return None;
        }
        let mut options = Options {
            view: View::Shell,
            settings: Settings::default(),
            administrator: true,
            image: None,
            snapshot: None,
            size: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "shell" => options.view = View::Shell,
                "settings" => options.view = View::Settings,
                "about" => options.view = View::About,
                "licences" | "licenses" => options.view = View::Licences,
                "gallery" => options.view = View::Gallery,
                "image" => options.view = View::Image,
                "drives" => options.view = View::Drives,
                "drives-none" => options.view = View::DrivesNone,
                "safety" => options.view = View::Safety,
                "safety-blocked" => options.view = View::SafetyBlocked,
                "windows-options" => options.view = View::WindowsOptions,
                "windows-options-set" => options.view = View::WindowsOptionsSet,
                "confirm-code" => options.view = View::ConfirmCode,
                "confirm-code-wrong" => options.view = View::ConfirmCodeWrong,
                "confirm-code-match" => options.view = View::ConfirmCodeMatch,
                "confirm-files" => options.view = View::ConfirmFiles,
                "confirm-files-ticked" => options.view = View::ConfirmFilesTicked,
                "confirm-phrase" => options.view = View::ConfirmPhrase,
                "confirm-expired" => options.view = View::ConfirmExpired,
                "writing" => options.view = View::Writing,
                "verifying" => options.view = View::Verifying,
                "done" => options.view = View::Done,
                "done-mismatch" => options.view = View::DoneMismatch,
                "--image" => options.image = args.next().map(PathBuf::from),
                "--theme" => match args.next().as_deref() {
                    Some("dark") => options.settings.theme = ThemeSetting::Dark,
                    Some("light") => options.settings.theme = ThemeSetting::Light,
                    _ => options.settings.theme = ThemeSetting::System,
                },
                "--not-admin" => options.administrator = false,
                "--verify-off" => options.settings.verify_after_writing = false,
                "--snapshot" => options.snapshot = args.next().map(PathBuf::from),
                "--size" => {
                    options.size = args.next().and_then(|size| {
                        let (width, height) = size.split_once('x')?;
                        Some((width.parse().ok()?, height.parse().ok()?))
                    })
                }
                other => eprintln!("--preview: ignoring unknown option {other}"),
            }
        }
        Some(options)
    }
}

pub fn run(options: Options) -> Result<(), slint::PlatformError> {
    if options.view == View::Gallery {
        let gallery = Gallery::new()?;
        gallery
            .global::<Theme>()
            .set_choice(shell::theme_choice(options.settings.theme));
        return present(gallery, options.snapshot);
    }

    let window = AppWindow::new()?;
    let drives = match options.view {
        View::DrivesNone => sample_drives()
            .into_iter()
            .filter(|drive| drive.status == TargetAssessmentStatus::Blocked)
            .take(2)
            .collect(),
        View::SafetyBlocked => small_stick_drives(),
        View::ConfirmPhrase => unreadable_card_drives(),
        _ => sample_drives(),
    };
    let shell = Shell::install_with(
        &window,
        options.settings,
        None,
        options.administrator,
        EngineSetup {
            engine: Arc::new(SampleEngine {
                drives,
                expired: options.view == View::ConfirmExpired,
            }),
            jobs: Jobs::Inline,
            clock: || "13:28".to_string(),
        },
    );
    let studio = window.global::<Studio>();
    let session = shell.borrow().session();
    if let Some(image) = &options.image {
        session.borrow_mut().open_image(image.clone());
    }
    match options.view {
        View::Settings => studio.set_settings_open(true),
        View::About => studio.set_about_open(true),
        View::Licences => {
            studio.set_about_open(true);
            studio.set_showing_licences(true);
        }
        View::Drives | View::DrivesNone | View::Safety | View::SafetyBlocked => {
            let mut session = session.borrow_mut();
            session.next();
            match options.view {
                View::Drives => session.select_drive(0),
                View::Safety => {
                    session.select_drive(0);
                    session.next();
                }
                View::SafetyBlocked => {
                    session.select_drive(1);
                    session.next();
                }
                _ => {}
            }
        }
        View::Writing | View::Verifying | View::Done | View::DoneMismatch => {
            let mut session = session.borrow_mut();
            if matches!(options.view, View::Done | View::DoneMismatch) {
                let expected = if options.view == View::Done {
                    SAMPLE_SHA256
                } else {
                    "A1ACC315D73B2A9CDF67FECB16403D751C99B6E50983382663FE0745CDFCDB13"
                };
                studio.set_expected_sha(expected.into());
                session.expected_sha_edited(expected);
            }
            session.next();
            session.select_drive(0);
            session.next();
            session.next();
            for option in 0..3 {
                session.set_option(option, true);
            }
            studio.set_account_name("alex".into());
            session.account_name_edited("alex");
            session.next();
            studio.set_typed("7F3A".into());
            session.typed_edited("7F3A");
            match options.view {
                View::Writing => session.show_write_progress(
                    RawWriteProgress {
                        phase: RawWritePhase::Writing,
                        completed_bytes: 4_155_000_000,
                        total_bytes: SAMPLE_IMAGE_BYTES,
                    },
                    197.0,
                ),
                View::Verifying => {
                    session.show_write_progress(
                        RawWriteProgress {
                            phase: RawWritePhase::Writing,
                            completed_bytes: SAMPLE_IMAGE_BYTES,
                            total_bytes: SAMPLE_IMAGE_BYTES,
                        },
                        451.0,
                    );
                    session.show_write_progress(
                        RawWriteProgress {
                            phase: RawWritePhase::Verifying,
                            completed_bytes: 2_560_000_000,
                            total_bytes: SAMPLE_IMAGE_BYTES,
                        },
                        125.0,
                    );
                }
                _ => session.write(),
            }
        }
        View::WindowsOptions
        | View::WindowsOptionsSet
        | View::ConfirmCode
        | View::ConfirmCodeWrong
        | View::ConfirmCodeMatch
        | View::ConfirmExpired => {
            let mut session = session.borrow_mut();
            session.next();
            session.select_drive(0);
            session.next();
            session.next();
            if options.view != View::WindowsOptions {
                for option in 0..3 {
                    session.set_option(option, true);
                }
                studio.set_account_name("alex".into());
                session.account_name_edited("alex");
            }
            if !matches!(options.view, View::WindowsOptions | View::WindowsOptionsSet) {
                session.next();
            }
            let typed = match options.view {
                View::ConfirmCodeWrong => "7F3B",
                View::ConfirmCodeMatch => "7F3A",
                _ => "",
            };
            studio.set_typed(typed.into());
            session.typed_edited(typed);
        }
        View::ConfirmFiles | View::ConfirmFilesTicked => {
            let mut session = session.borrow_mut();
            session.next();
            session.select_drive(2);
            session.next();
            session.next();
            studio.set_typed("B21C".into());
            session.typed_edited("B21C");
            if options.view == View::ConfirmFilesTicked {
                studio.set_acknowledged(true);
                session.acknowledged_changed(true);
            }
        }
        View::ConfirmPhrase => {
            let mut session = session.borrow_mut();
            session.next();
            session.select_drive(2);
            session.next();
            session.next();
            studio.set_typed("OSFLUX ERASE TARGET 4 ".into());
            session.typed_edited("OSFLUX ERASE TARGET 4 ");
        }
        View::Shell | View::Gallery | View::Image => {}
    }
    if let Some((width, height)) = options.size {
        window
            .window()
            .set_size(slint::LogicalSize::new(width as f32, height as f32));
    }
    present(window, options.snapshot)
}

/// Reads images with the real engine; the drives are the samples below.
struct SampleEngine {
    drives: Vec<TargetAssessment>,
    /// Confirmations are made already expired, for frame 21.
    expired: bool,
}

const PREVIEW_TIME: u64 = 1_800_000_000;

impl Engine for SampleEngine {
    /// A finished write as the design's frames show it. Nothing is written.
    fn write(
        &self,
        request: &WriteRequest,
        _cancel: &CancelToken,
        _progress: &mut dyn FnMut(RawWriteProgress),
    ) -> Result<RawWriteReport, WriteFailure> {
        let verify = request.verify;
        Ok(RawWriteReport {
            session_id: "OSFLUX-SESSION-PREVIEW".to_string(),
            image_path: request.image.display().to_string(),
            target_path: format!("\\\\.\\PhysicalDrive{}", request.target_index),
            target_identity_code: request.typed.clone(),
            image_sha256: SAMPLE_SHA256.to_ascii_lowercase(),
            bytes_written: SAMPLE_IMAGE_BYTES,
            bytes_verified: if verify { SAMPLE_IMAGE_BYTES } else { 0 },
            verification: if verify {
                Verification::ReadBack
            } else {
                Verification::Skipped
            },
            forced_dismount: false,
            write_millis: 451_000,
            verify_millis: if verify { 451_000 } else { 0 },
            audit_recorded: true,
        })
    }

    fn analyze_image(&self, path: &Path) -> Result<ImageAnalysis, String> {
        analyze_image(path)
    }

    fn scan_drives(&self) -> Vec<TargetAssessment> {
        self.drives.clone()
    }

    fn safety_check(
        &self,
        image: &Path,
        drive: &ChosenDrive,
    ) -> Result<SafetyCheck, SafetyFailure> {
        let plan = plan_image(image).map_err(|error| SafetyFailure::Image(error.to_string()))?;
        let target = self
            .drives
            .iter()
            .find(|target| target.index == drive.index)
            .cloned()
            .ok_or(SafetyFailure::DriveGone)?;
        Ok(build_safety_check(plan, target, false))
    }

    /// The engine's own intent for a sample drive. Nothing is recorded.
    fn confirm(&self, image: &Path, drive: &ChosenDrive) -> Result<Confirmation, ConfirmFailure> {
        let plan = plan_image(image).map_err(|error| ConfirmFailure::Failed(error.to_string()))?;
        let target = self
            .drives
            .iter()
            .find(|target| target.index == drive.index)
            .cloned()
            .ok_or(ConfirmFailure::DriveChanged)?;
        let intent = build_write_intent(&build_write_preflight(&plan, &target));
        if intent.status != WriteIntentStatus::ConfirmationRequired {
            return Err(ConfirmFailure::NotAllowed(intent.reason.clone()));
        }
        Ok(Confirmation {
            intent,
            expires_at_unix_seconds: if self.expired {
                PREVIEW_TIME
            } else {
                PREVIEW_TIME + 600
            },
        })
    }

    fn now_unix_seconds(&self) -> u64 {
        PREVIEW_TIME
    }
}

fn sample_drive(
    index: u32,
    model: &str,
    size: u64,
    bus: &str,
    media: &str,
    status: TargetAssessmentStatus,
    reason: &str,
) -> TargetAssessment {
    TargetAssessment {
        index,
        path: format!("\\\\.\\PhysicalDrive{index}"),
        status,
        reason: reason.to_string(),
        size_bytes: Some(size),
        model: Some(model.to_string()),
        interface_type: Some(bus.to_string()),
        media_type: Some(media.to_string()),
        serial_number: None,
        identity_code: Some(
            ["7F3A", "C41D", "7F3A", "93E0", "B21C", "5A07"][index as usize % 6].to_string(),
        ),
        access: DeviceAccess::MetadataOnly,
        contains_system_volume: index == 0,
        likely_removable: media == "Removable Media",
        requires_confirmation: status != TargetAssessmentStatus::Blocked,
        contents: None,
        notes: Vec::new(),
        warnings: Vec::new(),
    }
}

/// The drives of frame 05.
/// The samples, with the SD card unreadable, so it needs the full phrase.
fn unreadable_card_drives() -> Vec<TargetAssessment> {
    let mut drives = sample_drives();
    for drive in &mut drives {
        if drive.index == 4 {
            drive.contents = Some(DriveContents {
                state: DriveContentsState::Unknown,
                reason: "the drive could not be read".to_string(),
                partition_scheme: None,
                partitions: Vec::new(),
            });
        }
    }
    drives
}

fn sample_drives() -> Vec<TargetAssessment> {
    let removable = "Removable Media";
    let mut card = sample_drive(
        4,
        "SD card in USB card reader",
        15_931_539_456,
        "USB",
        removable,
        TargetAssessmentStatus::NeedsConfirmation,
        "target looks eligible but still requires explicit confirmation",
    );
    card.contents = Some(DriveContents {
        state: DriveContentsState::HasData,
        reason: "one FAT32 partition with files".to_string(),
        partition_scheme: None,
        partitions: vec![DrivePartition {
            number: 1,
            start_bytes: 1 << 20,
            size_bytes: 15_930_490_880,
            filesystem: Some(FilesystemKind::Fat32),
            label: Some("CARD".to_string()),
            used_bytes: Some(3_221_225_472),
        }],
    });
    vec![
        sample_drive(
            0,
            "Samsung SSD 980 1TB",
            1_000_204_886_016,
            "NVMe",
            "Fixed hard disk media",
            TargetAssessmentStatus::Blocked,
            "target contains the system volume",
        ),
        sample_drive(
            1,
            "Samsung T7 Shield 2TB",
            2_000_398_934_016,
            "USB",
            "External hard disk media",
            TargetAssessmentStatus::Blocked,
            "target is an external hard disk or SSD",
        ),
        sample_drive(
            2,
            "SanDisk Ultra 64GB",
            64_023_511_040,
            "USB",
            removable,
            TargetAssessmentStatus::Eligible,
            "target is empty, so its identity code is enough to confirm a write",
        ),
        sample_drive(
            3,
            "Kingston DataTraveler 32GB",
            31_037_849_600,
            "USB",
            removable,
            TargetAssessmentStatus::Eligible,
            "target is empty, so its identity code is enough to confirm a write",
        ),
        card,
        sample_drive(
            5,
            "Generic Flash Disk 8GB",
            8_053_063_680,
            "USB",
            "Fixed hard disk media",
            TargetAssessmentStatus::Blocked,
            "target does not report removable media",
        ),
    ]
}

/// Frame 08: an 8 GB stick in place of the 32 GB one.
fn small_stick_drives() -> Vec<TargetAssessment> {
    let mut drives = sample_drives();
    drives[3] = sample_drive(
        3,
        "Kingston DataTraveler 8GB",
        8_004_304_896,
        "USB",
        "Removable Media",
        TargetAssessmentStatus::Eligible,
        "target is empty, so its identity code is enough to confirm a write",
    );
    drives
}

fn present<C: ComponentHandle + 'static>(
    component: C,
    snapshot: Option<PathBuf>,
) -> Result<(), slint::PlatformError> {
    let Some(path) = snapshot else {
        return component.run();
    };
    component.show()?;
    let weak = component.as_weak();
    slint::Timer::single_shot(SNAPSHOT_DELAY, move || {
        if let Some(component) = weak.upgrade() {
            if let Err(error) = save_snapshot(component.window(), &path) {
                eprintln!("--snapshot: {error}");
            }
        }
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop()
}

fn save_snapshot(window: &slint::Window, path: &Path) -> Result<(), String> {
    let shot = window
        .take_snapshot()
        .map_err(|error| format!("could not take a snapshot: {error}"))?;
    image::save_buffer(
        path,
        shot.as_bytes(),
        shot.width(),
        shot.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|error| format!("could not save {}: {error}", path.display()))?;
    eprintln!(
        "saved {} ({}x{})",
        path.display(),
        shot.width(),
        shot.height()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Options, View};
    use crate::settings::ThemeSetting;

    fn parse(args: &[&str]) -> Option<Options> {
        Options::from_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn only_preview_arguments_start_a_preview() {
        assert!(parse(&[]).is_none());
        assert!(parse(&["--other"]).is_none());
        let options = parse(&["--preview"]).expect("a preview");
        assert_eq!(options.view, View::Shell);
        assert!(options.administrator);
    }

    #[test]
    fn reads_the_view_theme_and_states() {
        let options = parse(&[
            "--preview",
            "settings",
            "--theme",
            "light",
            "--not-admin",
            "--verify-off",
            "--snapshot",
            "out.png",
        ])
        .expect("a preview");
        assert_eq!(options.view, View::Settings);
        assert_eq!(options.settings.theme, ThemeSetting::Light);
        assert!(!options.settings.verify_after_writing);
        assert!(!options.administrator);
        assert_eq!(
            options.snapshot.as_deref(),
            Some(std::path::Path::new("out.png"))
        );
    }

    #[test]
    fn reads_the_step_views_and_the_image() {
        let options =
            parse(&["--preview", "safety-blocked", "--image", "a.iso"]).expect("a preview");
        assert_eq!(options.view, View::SafetyBlocked);
        assert_eq!(
            options.image.as_deref(),
            Some(std::path::Path::new("a.iso"))
        );
        for (name, view) in [
            ("image", View::Image),
            ("drives", View::Drives),
            ("drives-none", View::DrivesNone),
            ("safety", View::Safety),
        ] {
            assert_eq!(parse(&["--preview", name]).expect("a preview").view, view);
        }
    }
}
