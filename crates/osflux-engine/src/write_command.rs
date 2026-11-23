use std::fmt;
use std::path::Path;

use crate::preflight::{WritePreflightError, WritePreflightStatus};
use crate::write_intent::{
    create_write_intent, match_confirmation, WriteIntent, WriteIntentStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCommandPlan {
    pub status: WriteCommandStatus,
    pub reason: String,
    pub image_source: String,
    pub target_index: u32,
    pub target_path: String,
    pub target_identity_code: Option<String>,
    pub intent_status: WriteIntentStatus,
    pub expected_confirmation_phrase: Option<String>,
    pub supplied_confirmation_phrase_present: bool,
    pub confirmation_matches: bool,
    pub dry_run: bool,
    pub write_access_attempted: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl WriteCommandPlan {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"targetIdentityCode\": {},\n",
                "  \"intentStatus\": \"{}\",\n",
                "  \"expectedConfirmationPhrase\": {},\n",
                "  \"suppliedConfirmationPhrasePresent\": {},\n",
                "  \"confirmationMatches\": {},\n",
                "  \"dryRun\": {},\n",
                "  \"writeAccessAttempted\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            escape_json_string(&self.image_source),
            self.target_index,
            escape_json_string(&self.target_path),
            optional_string_json(self.target_identity_code.as_deref()),
            self.intent_status.as_str(),
            optional_string_json(self.expected_confirmation_phrase.as_deref()),
            self.supplied_confirmation_phrase_present,
            self.confirmation_matches,
            self.dry_run,
            self.write_access_attempted,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteCommandStatus {
    ReadyDryRun,
    ConfirmationMissing,
    ConfirmationMismatch,
    Blocked,
}

impl WriteCommandStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadyDryRun => "ready-dry-run",
            Self::ConfirmationMissing => "confirmation-missing",
            Self::ConfirmationMismatch => "confirmation-mismatch",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for WriteCommandStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn prepare_write_command(
    image_path: impl AsRef<Path>,
    target_index: u32,
    confirmation_phrase: Option<&str>,
) -> Result<WriteCommandPlan, WritePreflightError> {
    let intent = create_write_intent(image_path, target_index)?;

    Ok(build_write_command_plan(&intent, confirmation_phrase))
}

pub fn build_write_command_plan(
    intent: &WriteIntent,
    confirmation_phrase: Option<&str>,
) -> WriteCommandPlan {
    let supplied_confirmation_phrase_present = confirmation_phrase.is_some();
    let confirmation_matches = confirmation_phrase.is_some_and(|supplied| {
        match_confirmation(
            intent.confirmation_phrase.as_deref(),
            intent.short_confirmation.as_deref(),
            supplied,
        )
        .is_some()
    });
    let status = classify_write_command(intent, confirmation_phrase, confirmation_matches);
    let reason = write_command_reason(intent, status);

    let mut notes = Vec::new();
    notes.push(
        "Write command skeleton is a dry-run; no target drive was opened for write access."
            .to_string(),
    );
    notes.push(
        "The phrase is checked with exact string matching; an empty drive also accepts its identity code."
            .to_string(),
    );
    notes.push("The next milestone is wiring this plan into a locked write session.".to_string());

    let mut warnings = intent.warnings.clone();
    match status {
        WriteCommandStatus::ReadyDryRun => {
            warnings.push("A real write step is not enabled yet.".to_string());
        }
        WriteCommandStatus::ConfirmationMissing => {
            warnings.push("No confirmation phrase was supplied.".to_string());
        }
        WriteCommandStatus::ConfirmationMismatch => {
            warnings.push(
                "Supplied confirmation phrase did not match the generated phrase.".to_string(),
            );
        }
        WriteCommandStatus::Blocked => {}
    }

    WriteCommandPlan {
        status,
        reason: reason.to_string(),
        image_source: intent.image_source.clone(),
        target_index: intent.target_index,
        target_path: intent.target_path.clone(),
        target_identity_code: intent.target_identity_code.clone(),
        intent_status: intent.status,
        expected_confirmation_phrase: intent.confirmation_phrase.clone(),
        supplied_confirmation_phrase_present,
        confirmation_matches,
        dry_run: true,
        write_access_attempted: false,
        notes,
        warnings,
    }
}

fn classify_write_command(
    intent: &WriteIntent,
    confirmation_phrase: Option<&str>,
    confirmation_matches: bool,
) -> WriteCommandStatus {
    if intent.status == WriteIntentStatus::Blocked {
        return WriteCommandStatus::Blocked;
    }

    if confirmation_phrase.is_none() {
        return WriteCommandStatus::ConfirmationMissing;
    }

    if !confirmation_matches {
        return WriteCommandStatus::ConfirmationMismatch;
    }

    WriteCommandStatus::ReadyDryRun
}

fn write_command_reason(intent: &WriteIntent, status: WriteCommandStatus) -> &'static str {
    match status {
        WriteCommandStatus::ReadyDryRun => {
            "confirmation accepted; write execution is still disabled in this dry-run skeleton"
        }
        WriteCommandStatus::ConfirmationMissing => {
            "confirmation phrase is required before write command planning can continue"
        }
        WriteCommandStatus::ConfirmationMismatch => {
            "confirmation phrase did not match the expected write intent phrase"
        }
        WriteCommandStatus::Blocked => match intent.preflight_status {
            WritePreflightStatus::NeedsInspection => {
                "image needs deeper inspection before a write command can be planned"
            }
            WritePreflightStatus::Blocked => {
                "preflight is blocked and cannot enter a write command"
            }
            WritePreflightStatus::Ready | WritePreflightStatus::NeedsConfirmation => {
                "write intent is blocked and cannot enter a write command"
            }
        },
    }
}

fn optional_string_json(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", escape_json_string(value)),
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
    use super::{build_write_command_plan, WriteCommandStatus};
    use crate::preflight::WritePreflightStatus;
    use crate::write_intent::{WriteIntent, WriteIntentStatus};

    #[test]
    fn accepts_exact_confirmation_phrase_as_dry_run() {
        let plan = build_write_command_plan(
            &intent(WriteIntentStatus::ConfirmationRequired),
            Some("OSFLUX ERASE TARGET 2 7F3A"),
        );

        assert_eq!(plan.status, WriteCommandStatus::ReadyDryRun);
        assert!(plan.confirmation_matches);
        assert!(plan.dry_run);
        assert!(!plan.write_access_attempted);
    }

    #[test]
    fn rejects_missing_confirmation_phrase() {
        let plan = build_write_command_plan(&intent(WriteIntentStatus::ConfirmationRequired), None);

        assert_eq!(plan.status, WriteCommandStatus::ConfirmationMissing);
        assert!(!plan.supplied_confirmation_phrase_present);
        assert!(!plan.confirmation_matches);
    }

    #[test]
    fn rejects_mismatched_confirmation_phrase() {
        let plan = build_write_command_plan(
            &intent(WriteIntentStatus::ConfirmationRequired),
            Some("OSFLUX ERASE TARGET 3"),
        );

        assert_eq!(plan.status, WriteCommandStatus::ConfirmationMismatch);
        assert!(plan.supplied_confirmation_phrase_present);
        assert!(!plan.confirmation_matches);
    }

    #[test]
    fn rejects_phrase_confirmed_for_a_different_drive() {
        let plan = build_write_command_plan(
            &intent(WriteIntentStatus::ConfirmationRequired),
            Some("OSFLUX ERASE TARGET 2 0B21"),
        );

        assert_eq!(plan.status, WriteCommandStatus::ConfirmationMismatch);
    }

    #[test]
    fn rejects_phrase_without_identity_code() {
        let plan = build_write_command_plan(
            &intent(WriteIntentStatus::ConfirmationRequired),
            Some("OSFLUX ERASE TARGET 2"),
        );

        assert_eq!(plan.status, WriteCommandStatus::ConfirmationMismatch);
    }

    #[test]
    fn blocks_when_intent_is_blocked() {
        let plan = build_write_command_plan(
            &intent(WriteIntentStatus::Blocked),
            Some("OSFLUX ERASE TARGET 2 7F3A"),
        );

        assert_eq!(plan.status, WriteCommandStatus::Blocked);
        assert_eq!(plan.expected_confirmation_phrase, None);
        assert!(!plan.confirmation_matches);
        assert!(!plan.write_access_attempted);
    }

    #[test]
    fn serializes_write_command_plan_as_json() {
        let plan = build_write_command_plan(
            &intent(WriteIntentStatus::ConfirmationRequired),
            Some("OSFLUX ERASE TARGET 2 7F3A"),
        );
        let json = plan.to_json_pretty();

        assert!(json.contains("\"status\": \"ready-dry-run\""));
        assert!(json.contains("\"confirmationMatches\": true"));
        assert!(json.contains("\"writeAccessAttempted\": false"));
    }

    #[test]
    fn accepts_the_identity_code_only_when_the_intent_offers_it() {
        let mut empty_drive = intent(WriteIntentStatus::ConfirmationRequired);
        empty_drive.short_confirmation = Some("7F3A".to_string());

        let short = build_write_command_plan(&empty_drive, Some("7f3a"));
        let refused = build_write_command_plan(
            &intent(WriteIntentStatus::ConfirmationRequired),
            Some("7F3A"),
        );

        assert_eq!(short.status, WriteCommandStatus::ReadyDryRun);
        assert!(short.confirmation_matches);
        assert_eq!(refused.status, WriteCommandStatus::ConfirmationMismatch);
    }

    fn intent(status: WriteIntentStatus) -> WriteIntent {
        WriteIntent {
            status,
            reason: status.as_str().to_string(),
            confirmation_phrase: if status == WriteIntentStatus::ConfirmationRequired {
                Some("OSFLUX ERASE TARGET 2 7F3A".to_string())
            } else {
                None
            },
            short_confirmation: None,
            image_source: "C:\\images\\sample.iso".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            target_model: Some("Test Drive".to_string()),
            target_size_bytes: Some(32 * 1024 * 1024),
            target_identity_code: Some("7F3A".to_string()),
            preflight_status: if status == WriteIntentStatus::Blocked {
                WritePreflightStatus::Blocked
            } else {
                WritePreflightStatus::NeedsConfirmation
            },
            preflight_reason: "preflight result".to_string(),
            authorization_expires_at_unix_seconds: None,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
