//! Writing, verifying and Done (frames 15–20), and what a write that did
//! not finish says.
//!
//! The engine writes: `prepare_raw_write` checks the confirmation and the
//! drive once more, and `write_image_to_target` records the write in the
//! audit log, writes and reads back. Studio shows its progress, lets the
//! person cancel between chunks, and compares the image's SHA-256 with the
//! one they entered.

use std::path::PathBuf;

use osflux_engine::{
    AuthorizationProblem, DeviceError, RawWriteError, RawWritePhase, RawWriteProgress,
    RawWriteReport, SetupOptions, Sha256Hex, SuppliedConfirmation, TargetAssessmentError,
    Verification,
};

use crate::format::size_short;
use crate::{HintIcon, PhaseState, Status};

/// Everything the engine needs to write, gathered when Write to drive is
/// pressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    pub image: PathBuf,
    pub target_index: u32,
    /// Exactly what was typed: the code or the full phrase.
    pub typed: String,
    /// The engine's acknowledgement text, when the box naming what will be
    /// lost was ticked.
    pub acknowledgement: Option<String>,
    pub setup: SetupOptions,
    /// Copy this PC's region and keyboard, read when the stick is written.
    pub copy_region: bool,
    pub verify: bool,
}

impl WriteRequest {
    pub fn supplied(&self) -> SuppliedConfirmation<'_> {
        match &self.acknowledgement {
            Some(acknowledgement) => SuppliedConfirmation::CodeWithContentsAcknowledged {
                code: &self.typed,
                acknowledgement,
            },
            None => SuppliedConfirmation::Typed(&self.typed),
        }
    }
}

/// Why a write did not finish, in the terms Studio shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteFailure {
    /// The 10-minute confirmation ran out (frame 21).
    Expired,
    /// The drive is not the one that was confirmed, or what it holds
    /// changed (frame 22), with the engine's reason.
    DriveChanged(String),
    /// Studio is not running as Administrator (frame 23).
    NeedsAdministrator,
    /// Another program kept the drive open, so it could not be locked.
    InUse(String),
    /// Cancel was pressed. `written` is false when it stopped before
    /// anything reached the drive.
    Cancelled { written: bool },
    /// Anything else, with the engine's reason.
    Failed(String),
}

impl WriteFailure {
    pub fn from_error(error: &RawWriteError) -> Self {
        match error {
            RawWriteError::Authorization {
                problem: AuthorizationProblem::Expired { .. },
                ..
            } => Self::Expired,
            RawWriteError::Authorization {
                problem: AuthorizationProblem::TargetChanged,
                ..
            }
            | RawWriteError::TargetChanged { .. }
            | RawWriteError::ContentsAcknowledgementRefused { .. }
            | RawWriteError::ShortConfirmationRefused { .. }
            | RawWriteError::Target(TargetAssessmentError::NotFound { .. }) => {
                Self::DriveChanged(error.to_string())
            }
            RawWriteError::NeedsAdministrator { .. } => Self::NeedsAdministrator,
            RawWriteError::Open(DeviceError::InUse { .. }) => Self::InUse(error.to_string()),
            RawWriteError::Cancelled { phase, .. } => Self::Cancelled {
                written: *phase != RawWritePhase::Hashing,
            },
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Progress so far: the phase, its bytes, and the seconds spent in it.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteTracker {
    pub verify: bool,
    pub phase: RawWritePhase,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub phase_seconds: f64,
    /// Bytes per second while writing, kept for the verifying estimate.
    pub write_speed: Option<f64>,
    pub cancelling: bool,
}

impl WriteTracker {
    pub fn new(verify: bool) -> Self {
        Self {
            verify,
            phase: RawWritePhase::Writing,
            completed_bytes: 0,
            total_bytes: 0,
            phase_seconds: 0.0,
            write_speed: None,
            cancelling: false,
        }
    }

    /// Takes in one progress event, `phase_seconds` after its phase began.
    pub fn update(&mut self, progress: RawWriteProgress, phase_seconds: f64) {
        if self.phase == RawWritePhase::Writing && progress.phase == RawWritePhase::Verifying {
            self.write_speed = self.speed();
        }
        self.phase = progress.phase;
        self.completed_bytes = progress.completed_bytes;
        self.total_bytes = progress.total_bytes;
        self.phase_seconds = phase_seconds;
    }

    /// Bytes per second in the current phase, once there is enough to tell.
    fn speed(&self) -> Option<f64> {
        (self.phase_seconds >= 1.0 && self.completed_bytes > 0)
            .then(|| self.completed_bytes as f64 / self.phase_seconds)
    }

    fn seconds_left(&self) -> Option<f64> {
        self.speed()
            .map(|speed| self.total_bytes.saturating_sub(self.completed_bytes) as f64 / speed)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhaseView {
    pub title: &'static str,
    pub engine_phase: &'static str,
    pub description: String,
    pub state: PhaseState,
    pub percent: String,
    pub progress: f32,
    pub speed: String,
    pub done: String,
    pub time: String,
}

/// The Writing screen (frames 15–17).
#[derive(Debug, Clone, PartialEq)]
pub struct WritingView {
    pub title: String,
    pub description: &'static str,
    pub head: String,
    pub head_detail: &'static str,
    pub phases: Vec<PhaseView>,
    pub verify_off: bool,
    pub hint: &'static str,
    pub hint_icon: HintIcon,
    pub rail_subtitle: &'static str,
}

pub fn writing_view(tracker: &WriteTracker, model: &str) -> WritingView {
    let hashing = tracker.phase == RawWritePhase::Hashing;
    let writing = tracker.phase == RawWritePhase::Writing;
    let verifying = tracker.phase == RawWritePhase::Verifying;
    let total = size_short(tracker.total_bytes);
    let unknown = "—".to_string();

    let active = |description: String| PhaseView {
        title: "Writing",
        engine_phase: "writing",
        description,
        state: PhaseState::Active,
        percent: percent(tracker.completed_bytes, tracker.total_bytes),
        progress: fraction(tracker.completed_bytes, tracker.total_bytes),
        speed: tracker.speed().map_or(unknown.clone(), speed_text),
        done: format!("{} of {total}", size_short(tracker.completed_bytes)),
        time: tracker.seconds_left().map_or(unknown.clone(), time_left),
    };
    let write_phase = if hashing {
        PhaseView {
            engine_phase: "hashing",
            ..active("Reading the image first, to record its SHA-256.".to_string())
        }
    } else if writing {
        active(String::new())
    } else {
        PhaseView {
            title: "Writing",
            engine_phase: "writing",
            description: String::new(),
            state: PhaseState::Done,
            percent: "100%".to_string(),
            progress: 1.0,
            speed: tracker.write_speed.map_or(unknown.clone(), speed_text),
            done: format!("{total} of {total}"),
            time: "Done".to_string(),
        }
    };

    let mut phases = vec![write_phase];
    if tracker.verify {
        let verify_phase = if verifying {
            PhaseView {
                title: "Verifying",
                engine_phase: "verifying",
                ..active("Every byte is read back and compared.".to_string())
            }
        } else {
            // Waiting: estimated from the writing speed so far.
            let estimate = if writing {
                tracker
                    .speed()
                    .map(|speed| tracker.total_bytes as f64 / speed)
            } else {
                None
            };
            PhaseView {
                title: "Verifying",
                engine_phase: "verifying",
                description: "Every byte is read back and compared.".to_string(),
                state: PhaseState::Waiting,
                percent: "0%".to_string(),
                progress: 0.0,
                speed: unknown.clone(),
                done: if writing {
                    format!("0 GB of {total}")
                } else {
                    unknown.clone()
                },
                time: estimate.map_or(unknown.clone(), time_left),
            }
        };
        phases.push(verify_phase);
    }

    let seconds_left = match tracker.phase {
        RawWritePhase::Hashing => None,
        RawWritePhase::Writing => tracker.seconds_left().map(|left| {
            left + if tracker.verify {
                tracker
                    .speed()
                    .map_or(0.0, |speed| tracker.total_bytes as f64 / speed)
            } else {
                0.0
            }
        }),
        RawWritePhase::Verifying => tracker.seconds_left(),
    };
    let head = if tracker.cancelling {
        "Cancelling…".to_string()
    } else if hashing {
        "Reading the image".to_string()
    } else {
        match seconds_left {
            Some(seconds) => format!("{} left", time_left(seconds)),
            None => "Working out the time left".to_string(),
        }
    };

    WritingView {
        title: if verifying {
            format!("Verifying {model}")
        } else {
            format!("Writing to {model}")
        },
        description: if tracker.verify {
            "Keep the stick plugged in and OSflux open until both phases finish."
        } else {
            "Keep the stick plugged in and OSflux open until writing finishes."
        },
        head,
        head_detail: match (tracker.verify, verifying) {
            (true, false) => "Writing, then verifying",
            (true, true) => "Verifying, after writing",
            (false, _) => "Writing only · verification off",
        },
        phases,
        verify_off: !tracker.verify,
        hint: if tracker.cancelling {
            "Stopping after the current chunk…"
        } else {
            "If you cancel, the stick will need writing again before you can use it."
        },
        hint_icon: HintIcon::Warning,
        rail_subtitle: if verifying {
            "Verifying…"
        } else {
            "Writing…"
        },
    }
}

/// How a write ended, for the Done screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    Written(RawWriteReport),
    NotWritten(WriteFailure),
}

/// Which Done screen to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneKind {
    Verified,
    NotVerified,
    HashMismatch,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultRowView {
    pub key: String,
    pub key_detail: String,
    pub value: String,
    pub mono: bool,
    /// The value in red, for the hash that does not match.
    pub alert: bool,
    pub status: Status,
    pub status_text: String,
    pub detail: String,
}

impl ResultRowView {
    fn text(key: &str, value: impl Into<String>) -> Self {
        Self {
            key: key.to_string(),
            key_detail: String::new(),
            value: value.into(),
            mono: false,
            alert: false,
            status: Status::None,
            status_text: String::new(),
            detail: String::new(),
        }
    }

    fn mark(key: &str, status: Status, text: impl Into<String>) -> Self {
        Self {
            status,
            status_text: text.into(),
            ..Self::text(key, "")
        }
    }
}

/// The Done screen (frames 18–20), and the same screen for a write that
/// was cancelled or failed.
#[derive(Debug, Clone, PartialEq)]
pub struct DoneView {
    pub kind: DoneKind,
    pub status: Status,
    pub title: &'static str,
    pub description: String,
    /// Red or amber band under the title, when there is something to do.
    pub band_title: String,
    pub band_body: String,
    pub rows: Vec<ResultRowView>,
    pub next_step: &'static str,
    pub hint: &'static str,
    pub hint_icon: HintIcon,
    pub rail_subtitle: &'static str,
    pub rail_blocked: bool,
    /// "Write another stick", or "Try again" after a write that did not
    /// finish.
    pub primary: &'static str,
}

pub struct DoneInputs<'a> {
    pub outcome: &'a WriteOutcome,
    pub image_name: &'a str,
    pub model: &'a str,
    pub expected: Option<&'a Sha256Hex>,
    /// A Windows installer, which boots in UEFI mode only.
    pub uefi_only: bool,
}

const NEXT_UEFI: &str = "Next: plug the stick into the PC, open the boot menu (F12, F11 or Esc) and choose the UEFI: entry for the USB stick.";
const NEXT_ANY: &str =
    "Next: plug the stick into the PC, open the boot menu (F12, F11 or Esc) and choose the USB stick.";

pub fn done_view(inputs: DoneInputs<'_>) -> DoneView {
    match inputs.outcome {
        WriteOutcome::Written(report) => written_view(&inputs, report),
        WriteOutcome::NotWritten(failure) => not_written_view(failure),
    }
}

fn written_view(inputs: &DoneInputs<'_>, report: &RawWriteReport) -> DoneView {
    let actual = report.image_sha256.to_ascii_uppercase();
    let matches = inputs
        .expected
        .map(|expected| expected.matches(&report.image_sha256));
    let verified = report.verification == Verification::ReadBack;
    let audit = if report.audit_recorded {
        "Recorded in the audit log"
    } else {
        "Could not be added to the audit log"
    };

    if matches == Some(false) {
        let expected = inputs.expected.map(Sha256Hex::as_str).unwrap_or_default();
        let mut rows = vec![
            ResultRowView {
                key_detail: "from the download page".to_string(),
                mono: true,
                ..ResultRowView::text("Expected SHA-256", expected.to_ascii_uppercase())
            },
            ResultRowView {
                key_detail: "of this image file".to_string(),
                mono: true,
                alert: true,
                ..ResultRowView::text("Actual SHA-256", actual)
            },
            ResultRowView::mark("Result", Status::Blocked, "Mismatch"),
        ];
        rows.push(if verified {
            ResultRowView::mark(
                "Verification",
                Status::Ready,
                "Passed · the stick matches this image file",
            )
        } else {
            ResultRowView::mark(
                "Verification",
                Status::NeedsInspection,
                "Skipped · verify after writing was off",
            )
        });
        rows.push(ResultRowView::text(
            "Audit log",
            if report.audit_recorded {
                "Recorded in the audit log, with both hashes"
            } else {
                audit
            },
        ));
        return DoneView {
            kind: DoneKind::HashMismatch,
            status: Status::Blocked,
            title: "Hash mismatch",
            description: "The stick is an exact copy of the image file, but the file doesn't match the SHA-256 from the download page.".to_string(),
            band_title: "Download the image again before you use this stick.".to_string(),
            band_body: "The file may be damaged or changed. Delete it, download it again from the official page, then write another stick.".to_string(),
            rows,
            next_step: "",
            hint: "Recorded in the audit log.",
            hint_icon: HintIcon::Log,
            rail_subtitle: "Hash mismatch",
            rail_blocked: true,
            primary: "Write another stick",
        };
    }

    let mut rows = vec![ResultRowView {
        mono: true,
        ..ResultRowView::text("Image SHA-256", actual)
    }];
    rows.push(match inputs.expected {
        Some(expected) => ResultRowView {
            mono: true,
            status: Status::Ready,
            status_text: "Matches".to_string(),
            ..ResultRowView::text("Expected SHA-256", expected.as_str().to_ascii_uppercase())
        },
        None => ResultRowView::text("Expected SHA-256", "Not entered"),
    });
    rows.push(if verified {
        ResultRowView::mark(
            "Verification",
            Status::Ready,
            "Passed · every byte read back and compared",
        )
    } else {
        ResultRowView::mark(
            "Verification",
            Status::NeedsInspection,
            "Skipped · verify after writing was off",
        )
    });
    rows.push(ResultRowView {
        detail: if verified {
            format!(
                "writing {}, verifying {}",
                duration(report.write_millis),
                duration(report.verify_millis)
            )
        } else {
            String::new()
        },
        ..ResultRowView::text(
            "Time taken",
            duration(report.write_millis + report.verify_millis),
        )
    });
    rows.push(ResultRowView::text("Audit log", audit));

    let next_step = if inputs.uefi_only {
        NEXT_UEFI
    } else {
        NEXT_ANY
    };
    if verified {
        DoneView {
            kind: DoneKind::Verified,
            status: Status::Ready,
            title: "Written and verified",
            description: format!(
                "{} is on {}. Every byte was read back and matched.",
                inputs.image_name, inputs.model
            ),
            band_title: String::new(),
            band_body: String::new(),
            rows,
            next_step,
            hint: "Recorded in the audit log.",
            hint_icon: HintIcon::Log,
            rail_subtitle: "Written and verified",
            rail_blocked: false,
            primary: "Write another stick",
        }
    } else {
        DoneView {
            kind: DoneKind::NotVerified,
            status: Status::NeedsInspection,
            title: "Written, not verified",
            description: format!(
                "{} is on {}. Nothing was read back, so a fake or failing stick would not be noticed.",
                inputs.image_name, inputs.model
            ),
            band_title: String::new(),
            band_body: String::new(),
            rows,
            next_step,
            hint: "Recorded in the audit log.",
            hint_icon: HintIcon::Log,
            rail_subtitle: "Written, not verified",
            rail_blocked: false,
            primary: "Write another stick",
        }
    }
}

fn not_written_view(failure: &WriteFailure) -> DoneView {
    let (kind, title, band_title, band_body) = match failure {
        WriteFailure::Cancelled { written: false } => (
            DoneKind::Cancelled,
            "Writing cancelled",
            "Nothing was written to the drive.",
            "OSflux stopped while it was still reading the image. Confirm the drive again to write it.".to_string(),
        ),
        WriteFailure::Cancelled { written: true } => (
            DoneKind::Cancelled,
            "Writing cancelled",
            "This stick won't start a PC. It needs writing again before you can use it.",
            "Writing stopped part-way, so the stick holds only part of the image, and whatever was on it before is gone. Confirm the drive again to write it.".to_string(),
        ),
        WriteFailure::InUse(reason) => (
            DoneKind::Failed,
            "Not written",
            "Another program is using the drive.",
            format!("Close any program or window that has files open on it, then try again. Nothing was written. ({reason})"),
        ),
        WriteFailure::NeedsAdministrator => (
            DoneKind::Failed,
            "Not written",
            "Restart OSflux as Administrator to write.",
            "Windows only lets programs running as Administrator write to a drive. Nothing was written.".to_string(),
        ),
        WriteFailure::Expired | WriteFailure::DriveChanged(_) | WriteFailure::Failed(_) => (
            DoneKind::Failed,
            "Writing failed",
            "The stick may hold part of the image.",
            match failure {
                WriteFailure::Failed(reason) | WriteFailure::DriveChanged(reason) => {
                    format!("Write it again before you use it. The engine said: {reason}.")
                }
                _ => "Write it again before you use it.".to_string(),
            },
        ),
    };
    let description = match failure {
        WriteFailure::Cancelled { written: true } => "Writing stopped part-way.",
        WriteFailure::Cancelled { written: false } => "Writing stopped before it began.",
        _ => "",
    };
    let status = if kind == DoneKind::Cancelled {
        Status::NeedsInspection
    } else {
        Status::Blocked
    };
    DoneView {
        kind,
        status,
        title,
        description: description.to_string(),
        band_title: band_title.to_string(),
        band_body,
        rows: vec![ResultRowView::text(
            "Audit log",
            "Recorded in the audit log",
        )],
        next_step: "",
        hint: "Recorded in the audit log.",
        hint_icon: HintIcon::Log,
        rail_subtitle: match failure {
            WriteFailure::Cancelled { written: false } => "Cancelled",
            WriteFailure::Cancelled { written: true } => "Needs writing again",
            _ => "Not written",
        },
        rail_blocked: true,
        primary: "Try again",
    }
}

fn fraction(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

fn percent(done: u64, total: u64) -> String {
    format!("{}%", (fraction(done, total) * 100.0).floor() as u32)
}

fn speed_text(bytes_per_second: f64) -> String {
    format!("{}/s", size_short(bytes_per_second as u64))
}

/// "About 4 min", rounded up; "Less than a minute" under 60 seconds.
pub fn time_left(seconds: f64) -> String {
    if seconds < 60.0 {
        return "Less than a minute".to_string();
    }
    let minutes = (seconds / 60.0).ceil() as u64;
    if minutes < 60 {
        format!("About {minutes} min")
    } else {
        format!("About {} h {:02} min", minutes / 60, minutes % 60)
    }
}

/// "15 min 02 s", "42 s", "1 h 03 min".
pub fn duration(millis: u64) -> String {
    let seconds = millis.div_ceil(1000);
    if seconds < 60 {
        format!("{seconds} s")
    } else if seconds < 3600 {
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    } else {
        format!("{} h {:02} min", seconds / 3600, (seconds % 3600) / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;
    const IMAGE: u64 = 9_030_000_000;

    fn progress(phase: RawWritePhase, done: u64) -> RawWriteProgress {
        RawWriteProgress {
            phase,
            completed_bytes: done,
            total_bytes: IMAGE,
        }
    }

    fn report(verification: Verification) -> RawWriteReport {
        RawWriteReport {
            session_id: "S".into(),
            image_path: "Win11.iso".into(),
            target_path: "\\\\.\\PhysicalDrive2".into(),
            target_identity_code: "7F3A".into(),
            image_sha256: "2a1ab0d46d7285f738d082858925d3303f3dc430f0a4eab5aa238325a4274906".into(),
            bytes_written: IMAGE,
            bytes_verified: if verification == Verification::ReadBack {
                IMAGE
            } else {
                0
            },
            verification,
            forced_dismount: false,
            write_millis: 451_000,
            verify_millis: if verification == Verification::ReadBack {
                451_000
            } else {
                0
            },
            audit_recorded: true,
        }
    }

    #[test]
    fn writing_shows_both_phases_as_in_frame_15() {
        let mut tracker = WriteTracker::new(true);
        // 3.87 GB in 192 s: about 20 MB/s.
        tracker.update(progress(RawWritePhase::Writing, 4_155_000_000), 197.0);
        let view = writing_view(&tracker, "SanDisk Ultra 64GB");

        assert_eq!(view.title, "Writing to SanDisk Ultra 64GB");
        assert_eq!(view.head_detail, "Writing, then verifying");
        assert_eq!(view.phases.len(), 2);
        let writing = &view.phases[0];
        assert_eq!(writing.state, PhaseState::Active);
        assert_eq!(writing.percent, "46%");
        assert_eq!(writing.speed, "20.1 MB/s");
        assert_eq!(writing.done, "3.87 GB of 8.41 GB");
        assert_eq!(writing.time, "About 4 min");
        let verifying = &view.phases[1];
        assert_eq!(verifying.state, PhaseState::Waiting);
        assert_eq!(verifying.done, "0 GB of 8.41 GB");
        assert_eq!(verifying.time, "About 8 min");
        assert_eq!(view.head, "About 11 min left");
        assert_eq!(view.rail_subtitle, "Writing…");
        assert!(!view.verify_off);
    }

    #[test]
    fn verifying_marks_writing_done_as_in_frame_16() {
        let mut tracker = WriteTracker::new(true);
        tracker.update(progress(RawWritePhase::Writing, IMAGE / 2), 200.0);
        tracker.update(progress(RawWritePhase::Verifying, IMAGE / 4), 100.0);
        let view = writing_view(&tracker, "SanDisk Ultra 64GB");

        assert_eq!(view.title, "Verifying SanDisk Ultra 64GB");
        assert_eq!(view.phases[0].state, PhaseState::Done);
        assert_eq!(view.phases[0].percent, "100%");
        assert_eq!(view.phases[1].state, PhaseState::Active);
        assert_eq!(view.phases[1].percent, "25%");
        assert_eq!(view.rail_subtitle, "Verifying…");
    }

    #[test]
    fn without_verification_there_is_one_phase_as_in_frame_17() {
        let mut tracker = WriteTracker::new(false);
        tracker.update(progress(RawWritePhase::Writing, 0), 0.0);
        let view = writing_view(&tracker, "SanDisk Ultra 64GB");

        assert_eq!(view.phases.len(), 1);
        assert!(view.verify_off);
        assert_eq!(
            view.phases[0].speed, "—",
            "no speed before a second has passed"
        );
        assert_eq!(view.head, "Working out the time left");
        assert_eq!(view.head_detail, "Writing only · verification off");
    }

    #[test]
    fn reading_a_windows_iso_first_is_shown_on_the_writing_card() {
        let mut tracker = WriteTracker::new(true);
        tracker.update(progress(RawWritePhase::Hashing, 100 * MIB), 2.0);
        let view = writing_view(&tracker, "Stick");

        assert_eq!(view.head, "Reading the image");
        assert_eq!(view.phases[0].engine_phase, "hashing");
        assert!(view.phases[0].description.contains("SHA-256"));
    }

    #[test]
    fn cancelling_says_so() {
        let mut tracker = WriteTracker::new(true);
        tracker.update(progress(RawWritePhase::Writing, IMAGE / 3), 100.0);
        tracker.cancelling = true;
        let view = writing_view(&tracker, "Stick");
        assert_eq!(view.head, "Cancelling…");
        assert_eq!(view.hint, "Stopping after the current chunk…");
    }

    fn done(outcome: &WriteOutcome, expected: Option<&Sha256Hex>) -> DoneView {
        done_view(DoneInputs {
            outcome,
            image_name: "Windows11_Client_x64_en-gb_26300_9457.iso",
            model: "SanDisk Ultra 64GB",
            expected,
            uefi_only: true,
        })
    }

    #[test]
    fn written_and_verified_as_in_frame_18() {
        let expected =
            Sha256Hex::parse("2A1AB0D46D7285F738D082858925D3303F3DC430F0A4EAB5AA238325A4274906")
                .unwrap();
        let view = done(
            &WriteOutcome::Written(report(Verification::ReadBack)),
            Some(&expected),
        );

        assert_eq!(view.kind, DoneKind::Verified);
        assert_eq!(view.title, "Written and verified");
        assert_eq!(
            view.description,
            "Windows11_Client_x64_en-gb_26300_9457.iso is on SanDisk Ultra 64GB. Every byte was read back and matched."
        );
        assert_eq!(
            view.rows[0].value,
            "2A1AB0D46D7285F738D082858925D3303F3DC430F0A4EAB5AA238325A4274906"
        );
        assert_eq!(view.rows[1].status_text, "Matches");
        assert_eq!(
            view.rows[2].status_text,
            "Passed · every byte read back and compared"
        );
        assert_eq!(view.rows[3].value, "15 min 02 s");
        assert_eq!(
            view.rows[3].detail,
            "writing 7 min 31 s, verifying 7 min 31 s"
        );
        assert_eq!(view.rows[4].value, "Recorded in the audit log");
        assert!(view.next_step.contains("UEFI: entry"));
        assert!(!view.rail_blocked);
    }

    #[test]
    fn written_not_verified_as_in_frame_19() {
        let view = done(&WriteOutcome::Written(report(Verification::Skipped)), None);

        assert_eq!(view.kind, DoneKind::NotVerified);
        assert_eq!(view.status, Status::NeedsInspection);
        assert_eq!(view.rows[1].value, "Not entered");
        assert_eq!(view.rows[2].status, Status::NeedsInspection);
        assert_eq!(view.rows[3].value, "7 min 31 s");
        assert_eq!(view.rows[3].detail, "");
    }

    #[test]
    fn a_different_expected_hash_is_a_mismatch_as_in_frame_20() {
        let expected =
            Sha256Hex::parse("A1ACC315D73B2A9CDF67FECB16403D751C99B6E50983382663FE0745CDFCDB13")
                .unwrap();
        let view = done(
            &WriteOutcome::Written(report(Verification::ReadBack)),
            Some(&expected),
        );

        assert_eq!(view.kind, DoneKind::HashMismatch);
        assert_eq!(view.title, "Hash mismatch");
        assert_eq!(
            view.band_title,
            "Download the image again before you use this stick."
        );
        assert!(view.rows[1].alert, "the actual hash is shown in red");
        assert_eq!(view.rows[2].status_text, "Mismatch");
        assert_eq!(
            view.rows[3].status_text,
            "Passed · the stick matches this image file"
        );
        assert_eq!(
            view.rows[4].value,
            "Recorded in the audit log, with both hashes"
        );
        assert!(view.rail_blocked);
        assert_eq!(view.rail_subtitle, "Hash mismatch");
    }

    #[test]
    fn a_cancelled_write_says_whether_anything_was_written() {
        let partly = done(
            &WriteOutcome::NotWritten(WriteFailure::Cancelled { written: true }),
            None,
        );
        assert_eq!(partly.kind, DoneKind::Cancelled);
        assert_eq!(
            partly.band_title,
            "This stick won't start a PC. It needs writing again before you can use it."
        );
        assert_eq!(partly.description, "Writing stopped part-way.");
        assert_eq!(partly.rail_subtitle, "Needs writing again");
        assert!(partly.band_body.contains("only part of the image"));
        assert_eq!(partly.primary, "Try again");

        let before = done(
            &WriteOutcome::NotWritten(WriteFailure::Cancelled { written: false }),
            None,
        );
        assert_eq!(before.band_title, "Nothing was written to the drive.");
    }

    #[test]
    fn engine_errors_become_what_studio_shows() {
        let expired = RawWriteError::Authorization {
            target_index: 2,
            problem: AuthorizationProblem::Expired {
                expired_at_unix_seconds: 1,
            },
        };
        assert_eq!(WriteFailure::from_error(&expired), WriteFailure::Expired);
        assert!(matches!(
            WriteFailure::from_error(&RawWriteError::TargetChanged {
                reason: "size".into()
            }),
            WriteFailure::DriveChanged(_)
        ));
        assert_eq!(
            WriteFailure::from_error(&RawWriteError::NeedsAdministrator { target_index: 2 }),
            WriteFailure::NeedsAdministrator
        );
        assert!(matches!(
            WriteFailure::from_error(&RawWriteError::Open(DeviceError::InUse {
                volume: "E:".into(),
                message: "busy".into()
            })),
            WriteFailure::InUse(_)
        ));
        assert_eq!(
            WriteFailure::from_error(&RawWriteError::Cancelled {
                phase: RawWritePhase::Hashing,
                completed_bytes: 0,
                total_bytes: 1
            }),
            WriteFailure::Cancelled { written: false }
        );
        assert!(matches!(
            WriteFailure::from_error(&RawWriteError::VerifyMismatch { offset: 3 }),
            WriteFailure::Failed(_)
        ));
    }

    #[test]
    fn times_read_the_way_the_design_writes_them() {
        assert_eq!(time_left(30.0), "Less than a minute");
        assert_eq!(time_left(181.0), "About 4 min");
        assert_eq!(time_left(3_700.0), "About 1 h 02 min");
        assert_eq!(duration(42_000), "42 s");
        assert_eq!(duration(902_000), "15 min 02 s");
        assert_eq!(duration(3_780_000), "1 h 03 min");
    }

    #[test]
    fn the_request_supplies_the_acknowledgement_when_the_box_was_ticked() {
        let mut request = WriteRequest {
            image: "image.iso".into(),
            target_index: 4,
            typed: "B21C".into(),
            acknowledgement: None,
            setup: SetupOptions::default(),
            copy_region: false,
            verify: true,
        };
        assert_eq!(request.supplied(), SuppliedConfirmation::Typed("B21C"));
        request.acknowledgement = Some("Erase it".into());
        assert_eq!(
            request.supplied(),
            SuppliedConfirmation::CodeWithContentsAcknowledged {
                code: "B21C",
                acknowledgement: "Erase it",
            }
        );
    }
}
