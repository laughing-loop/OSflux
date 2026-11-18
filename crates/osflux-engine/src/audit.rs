use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::clock::{format_unix_rfc3339, now_unix_seconds};
use crate::json::{optional_number_json, optional_string_json, string_json};
use crate::raw_write::{RawWritePlan, RawWriteReport};
use crate::write_authorization::state_directory;
use crate::write_intent::WriteIntent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEventKind {
    Authorized,
    WriteRefused,
    WriteCancelled,
    WriteStarted,
    WriteSucceeded,
    WriteFailed,
}

impl AuditEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authorized => "authorized",
            Self::WriteRefused => "write-refused",
            Self::WriteCancelled => "write-cancelled",
            Self::WriteStarted => "write-started",
            Self::WriteSucceeded => "write-succeeded",
            Self::WriteFailed => "write-failed",
        }
    }
}

/// One line of the audit log: what happened, when, by whom, and to which
/// image and drive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub kind: AuditEventKind,
    pub unix_seconds: u64,
    pub user: Option<String>,
    pub host: Option<String>,
    pub session_id: Option<String>,
    pub image_path: String,
    pub image_size_bytes: Option<u64>,
    pub image_sha256: Option<String>,
    pub target_index: u32,
    pub target_path: Option<String>,
    pub target_model: Option<String>,
    pub target_identity_code: Option<String>,
    pub bytes_written: Option<u64>,
    pub detail: Option<String>,
}

impl AuditEvent {
    pub fn new(kind: AuditEventKind, image_path: impl Into<String>, target_index: u32) -> Self {
        Self {
            kind,
            unix_seconds: now_unix_seconds(),
            user: env_value(&["USERNAME", "USER"]),
            host: env_value(&["COMPUTERNAME", "HOSTNAME"]),
            session_id: None,
            image_path: image_path.into(),
            image_size_bytes: None,
            image_sha256: None,
            target_index,
            target_path: None,
            target_model: None,
            target_identity_code: None,
            bytes_written: None,
            detail: None,
        }
    }

    pub fn for_intent(kind: AuditEventKind, intent: &WriteIntent) -> Self {
        Self {
            target_path: Some(intent.target_path.clone()),
            target_model: intent.target_model.clone(),
            target_identity_code: intent.target_identity_code.clone(),
            ..Self::new(kind, intent.image_source.clone(), intent.target_index)
        }
    }

    pub fn for_plan(kind: AuditEventKind, plan: &RawWritePlan) -> Self {
        Self {
            session_id: Some(plan.session_id.clone()),
            image_size_bytes: Some(plan.image_size_bytes),
            target_path: Some(plan.target_path.clone()),
            target_model: plan.target_model.clone(),
            target_identity_code: Some(plan.target_identity_code.clone()),
            ..Self::new(
                kind,
                plan.image_path.display().to_string(),
                plan.target_index,
            )
        }
    }

    pub fn with_report(self, report: &RawWriteReport) -> Self {
        Self {
            image_sha256: Some(report.image_sha256.clone()),
            bytes_written: Some(report.bytes_written),
            ..self
        }
    }

    pub fn with_detail(self, detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..self
        }
    }

    /// The event as a single line of JSON, for JSON Lines tooling.
    pub fn to_json_line(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"time\":{},",
                "\"unixSeconds\":{},",
                "\"event\":{},",
                "\"user\":{},",
                "\"host\":{},",
                "\"osfluxVersion\":{},",
                "\"sessionId\":{},",
                "\"imagePath\":{},",
                "\"imageSizeBytes\":{},",
                "\"imageSha256\":{},",
                "\"targetIndex\":{},",
                "\"targetPath\":{},",
                "\"targetModel\":{},",
                "\"targetIdentityCode\":{},",
                "\"bytesWritten\":{},",
                "\"detail\":{}",
                "}}"
            ),
            string_json(&format_unix_rfc3339(self.unix_seconds)),
            self.unix_seconds,
            string_json(self.kind.as_str()),
            optional_string_json(self.user.as_deref()),
            optional_string_json(self.host.as_deref()),
            string_json(env!("CARGO_PKG_VERSION")),
            optional_string_json(self.session_id.as_deref()),
            string_json(&self.image_path),
            optional_number_json(self.image_size_bytes),
            optional_string_json(self.image_sha256.as_deref()),
            self.target_index,
            optional_string_json(self.target_path.as_deref()),
            optional_string_json(self.target_model.as_deref()),
            optional_string_json(self.target_identity_code.as_deref()),
            optional_number_json(self.bytes_written),
            optional_string_json(self.detail.as_deref()),
        )
    }
}

/// An append-only log of confirmations and writes, one JSON object per line.
#[derive(Debug, Clone)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `audit.log` in the OSflux state folder, when one can be found.
    pub fn default_location() -> Option<Self> {
        state_directory().map(|state| Self::new(state.join("audit.log")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends `event` and waits for it to reach the disk, so an entry that
    /// was reported as recorded survives a crash or power loss.
    pub fn record(&self, event: &AuditEvent) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(format!("{}\n", event.to_json_line()).as_bytes())?;
        file.sync_data()
    }
}

fn env_value(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::{AuditEvent, AuditEventKind, AuditLog};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn writes_one_json_object_per_line_with_escaped_values() {
        let mut event = AuditEvent::new(
            AuditEventKind::WriteFailed,
            r#"C:\Users\creer\Downloads\"odd" name.iso"#,
            3,
        )
        .with_detail("verification failed:\nbyte 17");
        event.unix_seconds = 1_795_022_016;

        let line = event.to_json_line();

        assert!(!line.contains('\n'));
        assert!(line.starts_with(
            r#"{"time":"2026-11-18T17:13:36Z","unixSeconds":1795022016,"event":"write-failed","#
        ));
        assert!(line.contains(r#""imagePath":"C:\\Users\\creer\\Downloads\\\"odd\" name.iso""#));
        assert!(line.contains(r#""detail":"verification failed:\nbyte 17""#));
        assert!(line.contains(r#""targetIndex":3,"#));
        assert!(line.contains(r#""imageSha256":null,"#));
    }

    #[test]
    fn appends_events_and_creates_the_folder() {
        let directory = temp_dir();
        let log = AuditLog::new(directory.join("nested").join("audit.log"));

        log.record(&AuditEvent::new(AuditEventKind::WriteStarted, "a.img", 3))
            .unwrap();
        log.record(&AuditEvent::new(AuditEventKind::WriteSucceeded, "a.img", 3))
            .unwrap();

        let contents = fs::read_to_string(log.path()).unwrap();
        let _ = fs::remove_dir_all(&directory);
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(r#""event":"write-started""#));
        assert!(lines[1].contains(r#""event":"write-succeeded""#));
    }

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "osflux-audit-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
