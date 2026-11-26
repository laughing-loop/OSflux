//! The Confirm step (frames 11–14, and 21 when the confirmation expires).
//!
//! The engine decides which confirmation a drive needs: its four-character
//! code when it is empty, the full phrase otherwise. It records a one-time
//! confirmation that lasts 10 minutes, and checks what the person types.
//! Studio passes exactly what was typed (upper-cased, as the inputs show it)
//! and never turns a code into the phrase.

use osflux_engine::{
    match_confirmation, ConfirmationForm, DriveContents, DriveContentsState, WriteIntent,
};

use crate::{HintIcon, MatchState};

/// The engine's confirmation for the chosen drive and image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub intent: WriteIntent,
    /// When the recorded confirmation stops covering a write.
    pub expires_at_unix_seconds: u64,
}

/// Why no confirmation could be offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmFailure {
    /// The drive at the chosen position is not the drive that was chosen.
    DriveChanged,
    /// The engine no longer allows this write, with its reason.
    NotAllowed(String),
    /// The confirmation could not be recorded, or the engine failed.
    Failed(String),
}

impl Confirmation {
    /// True when the drive's code alone confirms the write (an empty drive).
    pub fn short(&self) -> bool {
        self.intent.short_confirmation.is_some()
    }

    pub fn code(&self) -> &str {
        self.intent
            .target_identity_code
            .as_deref()
            .unwrap_or_default()
    }

    pub fn phrase(&self) -> &str {
        self.intent
            .confirmation_phrase
            .as_deref()
            .unwrap_or_default()
    }

    /// Which accepted confirmation `typed` is, as the engine judges it.
    pub fn matches(&self, typed: &str) -> Option<ConfirmationForm> {
        match_confirmation(
            self.intent.confirmation_phrase.as_deref(),
            self.intent.short_confirmation.as_deref(),
            typed,
        )
    }

    pub fn expired(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at_unix_seconds
    }
}

/// Upper-cases what was typed, as the inputs show it. The code boxes also
/// drop spaces and keep four characters, so a pasted code fills them.
pub fn normalize_typed(text: &str, short: bool) -> String {
    let upper = text.to_ascii_uppercase();
    if short {
        upper
            .chars()
            .filter(|c| !c.is_whitespace())
            .take(4)
            .collect()
    } else {
        upper
    }
}

/// What the Confirm screen says for one confirmation and what was typed.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmView {
    pub short: bool,
    pub description: &'static str,
    pub code: String,
    /// The phrase without its last four characters, and those characters.
    pub phrase_head: String,
    pub phrase_code: String,
    pub match_state: MatchState,
    pub match_message: String,
    pub matched: Option<ConfirmationForm>,
    pub can_write: bool,
    /// Amber, above the phrase: why the full phrase is needed.
    pub contents_note: String,
    /// Red: what will be lost.
    pub erase_title: String,
    pub erase_body: String,
    pub hint: &'static str,
    pub hint_icon: HintIcon,
    pub rail_subtitle: &'static str,
}

pub struct Inputs<'a> {
    pub confirmation: &'a Confirmation,
    pub model: &'a str,
    pub path: &'a str,
    pub contents: Option<&'a DriveContents>,
    pub typed: &'a str,
    pub expired: bool,
    pub verify: bool,
}

pub fn confirm_view(inputs: Inputs<'_>) -> ConfirmView {
    let confirmation = inputs.confirmation;
    let short = confirmation.short();
    let typed = inputs.typed;
    let matched = confirmation.matches(typed);

    let (match_state, match_message) = if short {
        let count = typed.chars().count();
        if count == 0 {
            (
                MatchState::Empty,
                "Type all 4 characters to unlock Write to drive.".to_string(),
            )
        } else if count < 4 {
            (
                MatchState::Typing,
                "Type all 4 characters to unlock Write to drive.".to_string(),
            )
        } else if matched.is_some() {
            (MatchState::Match, format!("Matches {}", inputs.model))
        } else {
            (
                MatchState::Mismatch,
                "Code does not match this drive".to_string(),
            )
        }
    } else {
        let phrase = confirmation.phrase();
        if typed.is_empty() {
            (
                MatchState::Empty,
                "Type the phrase exactly as shown.".to_string(),
            )
        } else if matched.is_some() {
            (
                MatchState::Match,
                format!(
                    "Matches. This phrase belongs to {} ({}).",
                    inputs.model,
                    confirmation.code()
                ),
            )
        } else if phrase.starts_with(typed) {
            (MatchState::Typing, "Keep typing…".to_string())
        } else {
            (
                MatchState::Mismatch,
                "Does not match. Check each word and the last 4 characters.".to_string(),
            )
        }
    };

    let phrase = confirmation.phrase();
    let split = phrase.len().saturating_sub(confirmation.code().len());
    let (phrase_head, phrase_code) = phrase.split_at(split.min(phrase.len()));

    let unknown_contents = inputs
        .contents
        .is_none_or(|contents| contents.state == DriveContentsState::Unknown);
    let contents_note = if short {
        String::new()
    } else if unknown_contents {
        "OSflux couldn't check what's on this drive, so it asks for the full phrase.".to_string()
    } else {
        "This drive has files on it, so OSflux asks for the full phrase.".to_string()
    };
    let erase_body = match inputs.contents {
        _ if unknown_contents => {
            "OSflux couldn't read what's on it. Anything on it will be lost. This cannot be undone."
                .to_string()
        }
        Some(contents) if !contents.partitions.is_empty() => {
            let count = contents.partitions.len();
            let partitions = if count == 1 {
                "1 partition".to_string()
            } else {
                format!("{count} partitions")
            };
            format!(
                "It has {partitions} with files. All of them will be lost. This cannot be undone."
            )
        }
        _ => "It holds data. All of it will be lost. This cannot be undone.".to_string(),
    };

    let (hint, hint_icon) = if inputs.expired {
        (
            "Write to drive is locked until you confirm again.",
            HintIcon::Lock,
        )
    } else if inputs.verify {
        (
            "Every byte will be read back and compared after writing.",
            HintIcon::Shield,
        )
    } else {
        (
            "Verification: off. A fake or failing stick won't be caught.",
            HintIcon::Warning,
        )
    };

    ConfirmView {
        short,
        description: if short {
            "Check this is the right drive, then type its code to unlock Write to drive."
        } else {
            "Type the full phrase to unlock Write to drive."
        },
        code: confirmation.code().to_string(),
        phrase_head: phrase_head.to_string(),
        phrase_code: phrase_code.to_string(),
        match_state,
        match_message,
        matched,
        can_write: matched.is_some() && !inputs.expired,
        contents_note,
        erase_title: format!(
            "Everything on {} ({}) will be erased.",
            inputs.model, inputs.path
        ),
        erase_body,
        hint,
        hint_icon,
        rail_subtitle: if inputs.expired {
            "Confirmation expired"
        } else if short {
            "Type the drive code"
        } else {
            "Type the full phrase"
        },
    }
}

/// What the screen says when no confirmation could be offered.
pub fn failure_text(failure: &ConfirmFailure) -> String {
    match failure {
        ConfirmFailure::DriveChanged => {
            "The drive changed after the safety check: it was unplugged, or another drive took its place. Choose the drive again."
                .to_string()
        }
        ConfirmFailure::NotAllowed(reason) => {
            format!("The engine no longer allows this write: {reason}. Go back to the safety check.")
        }
        ConfirmFailure::Failed(error) => format!("OSflux couldn't prepare the confirmation: {error}."),
    }
}

#[cfg(test)]
mod tests {
    use super::{confirm_view, normalize_typed, Confirmation, Inputs};
    use crate::{HintIcon, MatchState};
    use osflux_engine::{
        confirmation_phrase, ConfirmationForm, DriveContents, DriveContentsState, DrivePartition,
        WriteIntent, WriteIntentStatus, WritePreflightStatus,
    };

    fn confirmation(index: u32, code: &str, short: bool) -> Confirmation {
        Confirmation {
            intent: WriteIntent {
                status: WriteIntentStatus::ConfirmationRequired,
                reason: "ready".into(),
                confirmation_phrase: Some(confirmation_phrase(index, code)),
                short_confirmation: short.then(|| code.to_string()),
                image_source: "image.iso".into(),
                target_index: index,
                target_path: format!("\\\\.\\PhysicalDrive{index}"),
                target_model: Some("SanDisk Ultra 64GB".into()),
                target_size_bytes: Some(64_023_511_040),
                target_identity_code: Some(code.into()),
                preflight_status: WritePreflightStatus::Ready,
                preflight_reason: "ready".into(),
                authorization_expires_at_unix_seconds: Some(1_000),
                notes: Vec::new(),
                warnings: Vec::new(),
            },
            expires_at_unix_seconds: 1_000,
        }
    }

    fn view(confirmation: &Confirmation, typed: &str) -> super::ConfirmView {
        confirm_view(Inputs {
            confirmation,
            model: "SanDisk Ultra 64GB",
            path: "\\\\.\\PhysicalDrive2",
            contents: None,
            typed,
            expired: false,
            verify: true,
        })
    }

    #[test]
    fn an_empty_drive_takes_its_code_as_in_frames_11_to_13() {
        let confirmation = confirmation(2, "7F3A", true);
        let empty = view(&confirmation, "");
        assert!(empty.short);
        assert_eq!(empty.code, "7F3A");
        assert_eq!(empty.match_state, MatchState::Empty);
        assert_eq!(
            empty.match_message,
            "Type all 4 characters to unlock Write to drive."
        );
        assert!(!empty.can_write);
        assert_eq!(empty.hint_icon, HintIcon::Shield);
        assert_eq!(empty.rail_subtitle, "Type the drive code");

        let wrong = view(&confirmation, "7F3B");
        assert_eq!(wrong.match_state, MatchState::Mismatch);
        assert_eq!(wrong.match_message, "Code does not match this drive");
        assert!(!wrong.can_write);

        let right = view(&confirmation, "7F3A");
        assert_eq!(right.match_state, MatchState::Match);
        assert_eq!(right.match_message, "Matches SanDisk Ultra 64GB");
        assert_eq!(right.matched, Some(ConfirmationForm::IdentityCode));
        assert!(right.can_write);
    }

    #[test]
    fn a_drive_with_files_needs_the_full_phrase_as_in_frame_14() {
        let confirmation = confirmation(4, "B21C", false);
        let mut contents = DriveContents {
            state: DriveContentsState::HasData,
            reason: "files".into(),
            partition_scheme: None,
            partitions: vec![DrivePartition {
                number: 1,
                start_bytes: 1 << 20,
                size_bytes: 1 << 30,
                filesystem: None,
                label: None,
                used_bytes: None,
            }],
        };
        let make = |typed: &str, contents: &DriveContents| {
            confirm_view(Inputs {
                confirmation: &confirmation,
                model: "SD card in USB card reader",
                path: "\\\\.\\PhysicalDrive4",
                contents: Some(contents),
                typed,
                expired: false,
                verify: false,
            })
        };
        let typing = make("OSFLUX ERASE TARGET 4", &contents);
        assert!(!typing.short);
        assert_eq!(typing.phrase_head, "OSFLUX ERASE TARGET 4 ");
        assert_eq!(typing.phrase_code, "B21C");
        assert_eq!(typing.match_state, MatchState::Typing);
        assert_eq!(typing.match_message, "Keep typing…");
        assert_eq!(
            typing.contents_note,
            "This drive has files on it, so OSflux asks for the full phrase."
        );
        assert_eq!(
            typing.erase_title,
            "Everything on SD card in USB card reader (\\\\.\\PhysicalDrive4) will be erased."
        );
        assert_eq!(
            typing.erase_body,
            "It has 1 partition with files. All of them will be lost. This cannot be undone."
        );
        assert_eq!(typing.hint_icon, HintIcon::Warning);
        assert_eq!(typing.rail_subtitle, "Type the full phrase");

        // The code alone is not enough for a drive with files.
        assert_eq!(make("B21C", &contents).match_state, MatchState::Mismatch);
        assert_eq!(
            make("OSFLUX ERASE TARGET 4 B12C", &contents).match_state,
            MatchState::Mismatch
        );
        let right = make("OSFLUX ERASE TARGET 4 B21C", &contents);
        assert_eq!(right.match_state, MatchState::Match);
        assert_eq!(right.matched, Some(ConfirmationForm::FullPhrase));
        assert!(right.can_write);

        contents.state = DriveContentsState::Unknown;
        contents.partitions.clear();
        let unknown = make("", &contents);
        assert!(unknown.contents_note.starts_with("OSflux couldn't check"));
        assert!(unknown.erase_body.starts_with("OSflux couldn't read"));
    }

    #[test]
    fn an_expired_confirmation_locks_the_write_as_in_frame_21() {
        let confirmation = confirmation(2, "7F3A", true);
        assert!(confirmation.expired(1_000));
        assert!(!confirmation.expired(999));
        let view = confirm_view(Inputs {
            confirmation: &confirmation,
            model: "SanDisk Ultra 64GB",
            path: "\\\\.\\PhysicalDrive2",
            contents: None,
            typed: "7F3A",
            expired: true,
            verify: true,
        });
        assert!(
            !view.can_write,
            "a matching code is not enough once expired"
        );
        assert_eq!(
            view.hint,
            "Write to drive is locked until you confirm again."
        );
        assert_eq!(view.hint_icon, HintIcon::Lock);
        assert_eq!(view.rail_subtitle, "Confirmation expired");
    }

    #[test]
    fn typing_is_upper_cased_and_codes_keep_four_characters() {
        assert_eq!(normalize_typed("7f3a", true), "7F3A");
        assert_eq!(normalize_typed(" 7f 3a9 ", true), "7F3A");
        assert_eq!(
            normalize_typed("osflux erase target 4 b21c", false),
            "OSFLUX ERASE TARGET 4 B21C"
        );
    }
}
