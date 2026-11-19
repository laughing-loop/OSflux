use std::fmt;
use std::path::Path;

use crate::clock::now_unix_seconds;
use crate::fingerprint::stable_fingerprint;
use crate::preflight::WritePreflightError;
use crate::write_command::{prepare_write_command, WriteCommandPlan, WriteCommandStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteSession {
    pub session_id: String,
    pub status: WriteSessionStatus,
    pub reason: String,
    pub created_at_unix_seconds: u64,
    pub image_source: String,
    pub target_index: u32,
    pub target_path: String,
    pub target_identity_code: Option<String>,
    pub command_status: WriteCommandStatus,
    pub confirmed: bool,
    pub locked: bool,
    pub dry_run: bool,
    pub write_access_attempted: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl WriteSession {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"sessionId\": \"{}\",\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"createdAtUnixSeconds\": {},\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"targetIdentityCode\": {},\n",
                "  \"commandStatus\": \"{}\",\n",
                "  \"confirmed\": {},\n",
                "  \"locked\": {},\n",
                "  \"dryRun\": {},\n",
                "  \"writeAccessAttempted\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            escape_json_string(&self.session_id),
            self.status.as_str(),
            escape_json_string(&self.reason),
            self.created_at_unix_seconds,
            escape_json_string(&self.image_source),
            self.target_index,
            escape_json_string(&self.target_path),
            optional_string_json(self.target_identity_code.as_deref()),
            self.command_status.as_str(),
            self.confirmed,
            self.locked,
            self.dry_run,
            self.write_access_attempted,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteSessionStatus {
    Prepared,
    Blocked,
}

impl WriteSessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for WriteSessionStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn prepare_write_session(
    image_path: impl AsRef<Path>,
    target_index: u32,
    confirmation_phrase: Option<&str>,
) -> Result<WriteSession, WritePreflightError> {
    let command = prepare_write_command(image_path, target_index, confirmation_phrase)?;

    Ok(build_write_session(&command))
}

pub fn build_write_session(command: &WriteCommandPlan) -> WriteSession {
    build_write_session_at(command, now_unix_seconds())
}

pub fn build_write_session_at(
    command: &WriteCommandPlan,
    created_at_unix_seconds: u64,
) -> WriteSession {
    let status = classify_session_status(command.status);
    let confirmed = command.status == WriteCommandStatus::ReadyDryRun;
    let locked = confirmed;
    let session_id = build_session_id(command, created_at_unix_seconds);

    let mut notes = Vec::new();
    notes.push(
        "Write session locks the image and target identity from the confirmed command plan."
            .to_string(),
    );
    notes.push(
        "This session is a dry-run model; real write execution is still disabled.".to_string(),
    );
    notes.push("A future write backend must re-check session identity before writing.".to_string());

    let mut warnings = command.warnings.clone();
    if status == WriteSessionStatus::Prepared {
        warnings.push(
            "Prepared session is not proof that the target is still safe if devices change later."
                .to_string(),
        );
    }

    WriteSession {
        session_id,
        status,
        reason: session_reason(command.status).to_string(),
        created_at_unix_seconds,
        image_source: command.image_source.clone(),
        target_index: command.target_index,
        target_path: command.target_path.clone(),
        target_identity_code: command.target_identity_code.clone(),
        command_status: command.status,
        confirmed,
        locked,
        dry_run: true,
        write_access_attempted: false,
        notes,
        warnings,
    }
}

fn classify_session_status(command_status: WriteCommandStatus) -> WriteSessionStatus {
    match command_status {
        WriteCommandStatus::ReadyDryRun => WriteSessionStatus::Prepared,
        WriteCommandStatus::ConfirmationMissing
        | WriteCommandStatus::ConfirmationMismatch
        | WriteCommandStatus::Blocked => WriteSessionStatus::Blocked,
    }
}

fn session_reason(command_status: WriteCommandStatus) -> &'static str {
    match command_status {
        WriteCommandStatus::ReadyDryRun => {
            "write session prepared from a confirmed dry-run command"
        }
        WriteCommandStatus::ConfirmationMissing => {
            "write session blocked because confirmation phrase was missing"
        }
        WriteCommandStatus::ConfirmationMismatch => {
            "write session blocked because confirmation phrase did not match"
        }
        WriteCommandStatus::Blocked => "write session blocked by command planning",
    }
}

fn build_session_id(command: &WriteCommandPlan, created_at_unix_seconds: u64) -> String {
    let seed = format!(
        "{}|{}|{}|{}|{}",
        command.image_source,
        command.target_index,
        command.target_path,
        command.target_identity_code.as_deref().unwrap_or(""),
        created_at_unix_seconds
    );
    let fingerprint = stable_fingerprint(seed.as_bytes());

    format!(
        "OSFLUX-SESSION-{}-{}-{fingerprint:016X}",
        created_at_unix_seconds, command.target_index
    )
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
    use super::{build_write_session_at, WriteSessionStatus};
    use crate::write_command::{WriteCommandPlan, WriteCommandStatus};
    use crate::write_intent::WriteIntentStatus;

    #[test]
    fn prepares_session_for_ready_dry_run_command() {
        let session =
            build_write_session_at(&command(WriteCommandStatus::ReadyDryRun), 1_700_000_000);

        assert_eq!(session.status, WriteSessionStatus::Prepared);
        assert!(session.confirmed);
        assert!(session.locked);
        assert!(session.dry_run);
        assert!(!session.write_access_attempted);
        assert!(session
            .session_id
            .starts_with("OSFLUX-SESSION-1700000000-2-"));
    }

    #[test]
    fn session_id_changes_with_target_identity() {
        let first =
            build_write_session_at(&command(WriteCommandStatus::ReadyDryRun), 1_700_000_000);
        let mut other_drive = command(WriteCommandStatus::ReadyDryRun);
        other_drive.target_identity_code = Some("0B21".to_string());
        let second = build_write_session_at(&other_drive, 1_700_000_000);

        assert_ne!(first.session_id, second.session_id);
    }

    #[test]
    fn blocks_session_when_confirmation_is_missing() {
        let session = build_write_session_at(
            &command(WriteCommandStatus::ConfirmationMissing),
            1_700_000_000,
        );

        assert_eq!(session.status, WriteSessionStatus::Blocked);
        assert!(!session.confirmed);
        assert!(!session.locked);
    }

    #[test]
    fn blocks_session_when_confirmation_mismatches() {
        let session = build_write_session_at(
            &command(WriteCommandStatus::ConfirmationMismatch),
            1_700_000_000,
        );

        assert_eq!(session.status, WriteSessionStatus::Blocked);
        assert_eq!(
            session.reason,
            "write session blocked because confirmation phrase did not match"
        );
    }

    #[test]
    fn blocks_session_when_command_is_blocked() {
        let session = build_write_session_at(&command(WriteCommandStatus::Blocked), 1_700_000_000);

        assert_eq!(session.status, WriteSessionStatus::Blocked);
        assert_eq!(session.reason, "write session blocked by command planning");
    }

    #[test]
    fn serializes_session_as_json() {
        let session =
            build_write_session_at(&command(WriteCommandStatus::ReadyDryRun), 1_700_000_000);
        let json = session.to_json_pretty();

        assert!(json.contains("\"status\": \"prepared\""));
        assert!(json.contains("\"locked\": true"));
        assert!(json.contains("\"writeAccessAttempted\": false"));
    }

    fn command(status: WriteCommandStatus) -> WriteCommandPlan {
        WriteCommandPlan {
            status,
            reason: status.as_str().to_string(),
            image_source: "C:\\images\\sample.iso".to_string(),
            target_index: 2,
            target_path: "\\\\.\\PhysicalDrive2".to_string(),
            target_identity_code: Some("7F3A".to_string()),
            intent_status: if status == WriteCommandStatus::Blocked {
                WriteIntentStatus::Blocked
            } else {
                WriteIntentStatus::ConfirmationRequired
            },
            expected_confirmation_phrase: Some("OSFLUX ERASE TARGET 2 7F3A".to_string()),
            supplied_confirmation_phrase_present: status != WriteCommandStatus::ConfirmationMissing,
            confirmation_matches: status == WriteCommandStatus::ReadyDryRun,
            dry_run: true,
            write_access_attempted: false,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
