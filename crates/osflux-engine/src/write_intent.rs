use std::fmt;
use std::path::Path;

use crate::preflight::{
    run_write_preflight, WritePreflight, WritePreflightError, WritePreflightStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteIntent {
    pub status: WriteIntentStatus,
    pub reason: String,
    pub confirmation_phrase: Option<String>,
    pub image_source: String,
    pub target_index: u32,
    pub target_path: String,
    pub target_model: Option<String>,
    pub target_size_bytes: Option<u64>,
    pub target_identity_code: Option<String>,
    pub preflight_status: WritePreflightStatus,
    pub preflight_reason: String,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl WriteIntent {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"confirmationPhrase\": {},\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"targetModel\": {},\n",
                "  \"targetSizeBytes\": {},\n",
                "  \"targetIdentityCode\": {},\n",
                "  \"preflightStatus\": \"{}\",\n",
                "  \"preflightReason\": \"{}\",\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            optional_string_json(self.confirmation_phrase.as_deref()),
            escape_json_string(&self.image_source),
            self.target_index,
            escape_json_string(&self.target_path),
            optional_string_json(self.target_model.as_deref()),
            optional_number_json(self.target_size_bytes),
            optional_string_json(self.target_identity_code.as_deref()),
            self.preflight_status.as_str(),
            escape_json_string(&self.preflight_reason),
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteIntentStatus {
    ConfirmationRequired,
    Blocked,
}

impl WriteIntentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmationRequired => "confirmation-required",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for WriteIntentStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn create_write_intent(
    image_path: impl AsRef<Path>,
    target_index: u32,
) -> Result<WriteIntent, WritePreflightError> {
    let preflight = run_write_preflight(image_path, target_index)?;

    Ok(build_write_intent(&preflight))
}

pub fn build_write_intent(preflight: &WritePreflight) -> WriteIntent {
    let status = classify_intent_status(preflight.status);
    // The identity code ties the phrase to this drive, so a phrase confirmed
    // for one drive stops matching if a different drive takes the same index.
    let confirmation_phrase = match (status, preflight.target_identity_code.as_deref()) {
        (WriteIntentStatus::ConfirmationRequired, Some(identity_code)) => Some(format!(
            "OSFLUX ERASE TARGET {} {identity_code}",
            preflight.target_index
        )),
        _ => None,
    };

    let mut notes = Vec::new();
    notes.push("Write intent is read-only; no device writes were attempted.".to_string());
    notes.push(
        "Future write execution must require the exact confirmation phrase before opening the target for write access."
            .to_string(),
    );
    notes.push(
        "The phrase includes the target's identity code, so it stops matching if a different drive takes this index."
            .to_string(),
    );

    let mut warnings = preflight.warnings.clone();
    if status == WriteIntentStatus::ConfirmationRequired {
        warnings.push(
            "The selected target would be erased by a future confirmed write operation."
                .to_string(),
        );
    }

    WriteIntent {
        status,
        reason: intent_reason(preflight.status).to_string(),
        confirmation_phrase,
        image_source: preflight.image_source.clone(),
        target_index: preflight.target_index,
        target_path: preflight.target_path.clone(),
        target_model: preflight.target_model.clone(),
        target_size_bytes: preflight.target_size_bytes,
        target_identity_code: preflight.target_identity_code.clone(),
        preflight_status: preflight.status,
        preflight_reason: preflight.reason.clone(),
        notes,
        warnings,
    }
}

fn classify_intent_status(status: WritePreflightStatus) -> WriteIntentStatus {
    match status {
        WritePreflightStatus::Ready | WritePreflightStatus::NeedsConfirmation => {
            WriteIntentStatus::ConfirmationRequired
        }
        WritePreflightStatus::NeedsInspection | WritePreflightStatus::Blocked => {
            WriteIntentStatus::Blocked
        }
    }
}

fn intent_reason(status: WritePreflightStatus) -> &'static str {
    match status {
        WritePreflightStatus::Ready => {
            "preflight passed; explicit confirmation is still required before writing"
        }
        WritePreflightStatus::NeedsConfirmation => {
            "preflight passed and target confirmation is required before writing"
        }
        WritePreflightStatus::NeedsInspection => {
            "image needs deeper inspection before write intent can continue"
        }
        WritePreflightStatus::Blocked => "preflight is blocked and cannot create a write intent",
    }
}

fn optional_string_json(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", escape_json_string(value)),
        None => "null".to_string(),
    }
}

fn optional_number_json(value: Option<u64>) -> String {
    match value {
        Some(value) => value.to_string(),
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
    use super::{build_write_intent, WriteIntentStatus};
    use crate::image_plan::ImagePlanStatus;
    use crate::preflight::{WritePreflight, WritePreflightStatus};
    use crate::target::TargetAssessmentStatus;

    #[test]
    fn creates_confirmation_phrase_when_preflight_needs_confirmation() {
        let intent = build_write_intent(&preflight(WritePreflightStatus::NeedsConfirmation));

        assert_eq!(intent.status, WriteIntentStatus::ConfirmationRequired);
        assert_eq!(
            intent.confirmation_phrase.as_deref(),
            Some("OSFLUX ERASE TARGET 2 7F3A")
        );
    }

    #[test]
    fn phrase_changes_when_a_different_drive_takes_the_index() {
        let mut other_drive = preflight(WritePreflightStatus::NeedsConfirmation);
        other_drive.target_identity_code = Some("0B21".to_string());

        let first = build_write_intent(&preflight(WritePreflightStatus::NeedsConfirmation));
        let second = build_write_intent(&other_drive);

        assert_ne!(first.confirmation_phrase, second.confirmation_phrase);
    }

    #[test]
    fn withholds_phrase_when_target_identity_is_unknown() {
        let mut unknown_drive = preflight(WritePreflightStatus::NeedsConfirmation);
        unknown_drive.target_identity_code = None;

        let intent = build_write_intent(&unknown_drive);

        assert_eq!(intent.confirmation_phrase, None);
    }

    #[test]
    fn still_requires_confirmation_when_preflight_is_ready() {
        let intent = build_write_intent(&preflight(WritePreflightStatus::Ready));

        assert_eq!(intent.status, WriteIntentStatus::ConfirmationRequired);
        assert_eq!(
            intent.reason,
            "preflight passed; explicit confirmation is still required before writing"
        );
    }

    #[test]
    fn blocks_when_preflight_is_blocked() {
        let intent = build_write_intent(&preflight(WritePreflightStatus::Blocked));

        assert_eq!(intent.status, WriteIntentStatus::Blocked);
        assert_eq!(intent.confirmation_phrase, None);
    }

    #[test]
    fn blocks_when_preflight_needs_inspection() {
        let intent = build_write_intent(&preflight(WritePreflightStatus::NeedsInspection));

        assert_eq!(intent.status, WriteIntentStatus::Blocked);
        assert_eq!(
            intent.reason,
            "image needs deeper inspection before write intent can continue"
        );
    }

    #[test]
    fn serializes_intent_as_json() {
        let intent = build_write_intent(&preflight(WritePreflightStatus::NeedsConfirmation));
        let json = intent.to_json_pretty();

        assert!(json.contains("\"status\": \"confirmation-required\""));
        assert!(json.contains("\"confirmationPhrase\": \"OSFLUX ERASE TARGET 2 7F3A\""));
        assert!(json.contains("\"targetModel\": \"Test Drive\""));
        assert!(json.contains("\"targetIndex\": 2"));
    }

    fn preflight(status: WritePreflightStatus) -> WritePreflight {
        WritePreflight {
            status,
            reason: status.as_str().to_string(),
            image_source: "C:\\images\\sample.iso".to_string(),
            image_status: ImagePlanStatus::Ready,
            image_status_reason: "ready".to_string(),
            image_write_strategy: "iso-filesystem-boot-setup".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            target_model: Some("Test Drive".to_string()),
            target_size_bytes: Some(32 * 1024 * 1024),
            target_identity_code: Some("7F3A".to_string()),
            target_status: TargetAssessmentStatus::NeedsConfirmation,
            target_reason: "target looks eligible but still requires explicit confirmation"
                .to_string(),
            requires_confirmation: true,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
