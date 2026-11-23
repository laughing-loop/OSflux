use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::clock::format_unix_utc;
use crate::write_intent::{match_confirmation, ConfirmationForm, WriteIntent, WriteIntentStatus};

/// How long a confirmation issued by `--intent` stays usable.
pub const AUTHORIZATION_LIFETIME_SECONDS: u64 = 10 * 60;
/// Overrides where OSflux keeps its state, for managed machines and tests.
pub const STATE_DIR_ENV: &str = "OSFLUX_STATE_DIR";

// Version 2 added `short_confirmation`. Records only live for ten minutes, so
// an older record is simply refused and `--intent` run again.
const RECORD_HEADER: &str = "osflux-write-authorization 2";
/// Tolerates small clock corrections between issuing and checking.
const CLOCK_SKEW_SECONDS: u64 = 60;

/// Identifies an image file well enough to notice that it was replaced or
/// modified after a confirmation was issued for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageIdentity {
    pub path: String,
    pub size_bytes: u64,
    pub modified_unix_seconds: u64,
}

impl ImageIdentity {
    pub fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let canonical = fs::canonicalize(path)?;
        let metadata = fs::metadata(&canonical)?;
        let modified_unix_seconds = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        Ok(Self {
            path: canonical.display().to_string(),
            size_bytes: metadata.len(),
            modified_unix_seconds,
        })
    }
}

/// A one-time permission to erase one drive with one image, issued by
/// `--intent` and used up by `--write --execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteAuthorization {
    pub target_index: u32,
    pub target_path: String,
    pub target_identity_code: String,
    pub confirmation_phrase: String,
    /// The identity code, when the drive was empty at `--intent` and the code
    /// alone may confirm the write.
    pub short_confirmation: Option<String>,
    pub image: ImageIdentity,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorizationProblem {
    Missing,
    Expired {
        expired_at_unix_seconds: u64,
    },
    IssuedInFuture,
    PhraseMismatch,
    /// The identity code was typed, but the drive was not empty when the
    /// confirmation was issued, so only the full phrase covers it.
    ShortConfirmationNotRecorded,
    TargetChanged,
    ImageMismatch,
}

impl AuthorizationProblem {
    pub fn describe(&self, target_index: u32) -> String {
        match self {
            Self::Missing => format!(
                "no confirmation is active for drive {target_index}; run --intent first"
            ),
            Self::Expired {
                expired_at_unix_seconds,
            } => format!(
                "the confirmation for drive {target_index} expired at {}; run --intent again",
                format_unix_utc(*expired_at_unix_seconds)
            ),
            Self::IssuedInFuture => format!(
                "the confirmation for drive {target_index} is dated in the future; run --intent again"
            ),
            Self::PhraseMismatch => format!(
                "the confirmation phrase is not the one --intent recorded for drive {target_index}"
            ),
            Self::ShortConfirmationNotRecorded => format!(
                "drive {target_index} was not empty when --intent ran, so the write needs the full phrase; if the drive has been emptied since, run --intent again"
            ),
            Self::TargetChanged => format!(
                "drive {target_index} is not the drive the confirmation was issued for; run --intent again"
            ),
            Self::ImageMismatch => format!(
                "the confirmation for drive {target_index} was issued for a different image, or the image changed since; run --intent again"
            ),
        }
    }
}

impl WriteAuthorization {
    /// Builds the authorization for a write intent that is ready for
    /// confirmation, or `None` when the intent cannot be confirmed.
    pub fn issue(
        intent: &WriteIntent,
        image: ImageIdentity,
        now_unix_seconds: u64,
    ) -> Option<Self> {
        if intent.status != WriteIntentStatus::ConfirmationRequired {
            return None;
        }

        Some(Self {
            target_index: intent.target_index,
            target_path: intent.target_path.clone(),
            target_identity_code: intent.target_identity_code.clone()?,
            confirmation_phrase: intent.confirmation_phrase.clone()?,
            short_confirmation: intent.short_confirmation.clone(),
            image,
            issued_at_unix_seconds: now_unix_seconds,
            expires_at_unix_seconds: now_unix_seconds + AUTHORIZATION_LIFETIME_SECONDS,
        })
    }

    /// Checks that this authorization covers writing `image` to the drive
    /// described by the other arguments, right now, and says which accepted
    /// confirmation was typed.
    pub fn check(
        &self,
        target_index: u32,
        target_path: &str,
        target_identity_code: &str,
        image: &ImageIdentity,
        supplied_phrase: &str,
        now_unix_seconds: u64,
    ) -> Result<ConfirmationForm, AuthorizationProblem> {
        if self.issued_at_unix_seconds > now_unix_seconds + CLOCK_SKEW_SECONDS {
            return Err(AuthorizationProblem::IssuedInFuture);
        }

        if now_unix_seconds >= self.expires_at_unix_seconds {
            return Err(AuthorizationProblem::Expired {
                expired_at_unix_seconds: self.expires_at_unix_seconds,
            });
        }

        if self.target_index != target_index
            || self.target_path != target_path
            || self.target_identity_code != target_identity_code
        {
            return Err(AuthorizationProblem::TargetChanged);
        }

        let Some(form) = match_confirmation(
            Some(&self.confirmation_phrase),
            self.short_confirmation.as_deref(),
            supplied_phrase,
        ) else {
            if supplied_phrase
                .trim()
                .eq_ignore_ascii_case(&self.target_identity_code)
            {
                return Err(AuthorizationProblem::ShortConfirmationNotRecorded);
            }
            return Err(AuthorizationProblem::PhraseMismatch);
        };

        if &self.image != image {
            return Err(AuthorizationProblem::ImageMismatch);
        }

        Ok(form)
    }

    fn to_record(&self) -> Result<String, AuthorizationStoreError> {
        let fields = [
            ("target_index", self.target_index.to_string()),
            ("target_path", self.target_path.clone()),
            ("target_identity_code", self.target_identity_code.clone()),
            ("confirmation_phrase", self.confirmation_phrase.clone()),
            (
                "short_confirmation",
                self.short_confirmation.clone().unwrap_or_default(),
            ),
            ("image_path", self.image.path.clone()),
            ("image_size_bytes", self.image.size_bytes.to_string()),
            (
                "image_modified_unix_seconds",
                self.image.modified_unix_seconds.to_string(),
            ),
            (
                "issued_at_unix_seconds",
                self.issued_at_unix_seconds.to_string(),
            ),
            (
                "expires_at_unix_seconds",
                self.expires_at_unix_seconds.to_string(),
            ),
        ];

        let mut record = format!("{RECORD_HEADER}\n");
        for (key, value) in fields {
            // One field per line, so a value must not be able to start another.
            if value.contains(['\n', '\r']) {
                return Err(AuthorizationStoreError::Unrecordable {
                    reason: format!("{key} contains a line break"),
                });
            }
            record.push_str(&format!("{key}={value}\n"));
        }

        Ok(record)
    }

    fn from_record(record: &str) -> Result<Self, String> {
        let mut lines = record.lines();
        if lines.next() != Some(RECORD_HEADER) {
            return Err("unrecognised header".to_string());
        }

        let mut fields = std::collections::HashMap::new();
        for line in lines.filter(|line| !line.is_empty()) {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("malformed line: {line}"))?;
            if fields.insert(key, value).is_some() {
                return Err(format!("duplicate field: {key}"));
            }
        }

        let text = |key: &str| {
            fields
                .get(key)
                .map(|value| value.to_string())
                .ok_or_else(|| format!("missing field: {key}"))
        };
        let number = |key: &str| {
            text(key)?
                .parse::<u64>()
                .map_err(|_| format!("field is not a number: {key}"))
        };

        Ok(Self {
            target_index: u32::try_from(number("target_index")?)
                .map_err(|_| "target_index is out of range".to_string())?,
            target_path: text("target_path")?,
            target_identity_code: text("target_identity_code")?,
            confirmation_phrase: text("confirmation_phrase")?,
            short_confirmation: Some(text("short_confirmation")?).filter(|code| !code.is_empty()),
            image: ImageIdentity {
                path: text("image_path")?,
                size_bytes: number("image_size_bytes")?,
                modified_unix_seconds: number("image_modified_unix_seconds")?,
            },
            issued_at_unix_seconds: number("issued_at_unix_seconds")?,
            expires_at_unix_seconds: number("expires_at_unix_seconds")?,
        })
    }
}

/// Records a one-time confirmation for `intent` when it is ready for one,
/// and notes on the intent when the confirmation expires. Returns `None`
/// for an intent that cannot be confirmed.
pub fn record_write_authorization(
    intent: &mut WriteIntent,
    image_path: impl AsRef<Path>,
    store: &AuthorizationStore,
    now_unix_seconds: u64,
) -> Result<Option<WriteAuthorization>, AuthorizationStoreError> {
    if intent.status != WriteIntentStatus::ConfirmationRequired {
        return Ok(None);
    }

    let image_path = image_path.as_ref();
    let image = ImageIdentity::read(image_path).map_err(|source| AuthorizationStoreError::Io {
        path: image_path.to_path_buf(),
        source,
    })?;
    let Some(authorization) = WriteAuthorization::issue(intent, image, now_unix_seconds) else {
        return Ok(None);
    };

    store.save(&authorization)?;
    intent.authorization_expires_at_unix_seconds = Some(authorization.expires_at_unix_seconds);
    intent.notes.push(format!(
        "Confirmation recorded for one write until {}.",
        format_unix_utc(authorization.expires_at_unix_seconds)
    ));

    Ok(Some(authorization))
}

#[derive(Debug)]
pub enum AuthorizationStoreError {
    NoStateDirectory,
    Io { path: PathBuf, source: io::Error },
    Corrupt { path: PathBuf, reason: String },
    Unrecordable { reason: String },
}

impl fmt::Display for AuthorizationStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStateDirectory => write!(
                formatter,
                "could not find a folder for OSflux state; set {STATE_DIR_ENV}"
            ),
            Self::Io { path, source } => {
                write!(formatter, "could not access {}: {source}", path.display())
            }
            Self::Corrupt { path, reason } => write!(
                formatter,
                "the confirmation record {} is unreadable ({reason}); run --intent again",
                path.display()
            ),
            Self::Unrecordable { reason } => {
                write!(formatter, "the confirmation cannot be recorded: {reason}")
            }
        }
    }
}

impl std::error::Error for AuthorizationStoreError {}

/// Keeps at most one active authorization per drive index on disk.
#[derive(Debug, Clone)]
pub struct AuthorizationStore {
    directory: PathBuf,
}

impl AuthorizationStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn default_location() -> Result<Self, AuthorizationStoreError> {
        state_directory()
            .map(|state| Self::new(state.join("authorizations")))
            .ok_or(AuthorizationStoreError::NoStateDirectory)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Saves `authorization`, replacing any earlier one for the same drive.
    /// The record is written to a temporary file and renamed into place, so a
    /// crash never leaves a half-written record behind.
    pub fn save(&self, authorization: &WriteAuthorization) -> Result<(), AuthorizationStoreError> {
        let record = authorization.to_record()?;
        let io_error = |path: &Path| {
            let path = path.to_path_buf();
            move |source| AuthorizationStoreError::Io { path, source }
        };

        fs::create_dir_all(&self.directory).map_err(io_error(&self.directory))?;
        let final_path = self.record_path(authorization.target_index);
        let temporary_path = final_path.with_extension(format!("tmp-{}", std::process::id()));

        let mut file = fs::File::create(&temporary_path).map_err(io_error(&temporary_path))?;
        file.write_all(record.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(io_error(&temporary_path))?;
        drop(file);

        fs::rename(&temporary_path, &final_path).map_err(|source| {
            let _ = fs::remove_file(&temporary_path);
            AuthorizationStoreError::Io {
                path: final_path.clone(),
                source,
            }
        })
    }

    pub fn load(
        &self,
        target_index: u32,
    ) -> Result<Option<WriteAuthorization>, AuthorizationStoreError> {
        let path = self.record_path(target_index);
        let record = match fs::read_to_string(&path) {
            Ok(record) => record,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(AuthorizationStoreError::Io { path, source }),
        };

        WriteAuthorization::from_record(&record)
            .map(Some)
            .map_err(|reason| AuthorizationStoreError::Corrupt { path, reason })
    }

    /// Removes the authorization for a drive so it cannot be used again. Fails
    /// when there is nothing to remove, so two writes can never share one.
    pub fn consume(&self, target_index: u32) -> Result<(), AuthorizationStoreError> {
        let path = self.record_path(target_index);
        fs::remove_file(&path).map_err(|source| AuthorizationStoreError::Io { path, source })
    }

    fn record_path(&self, target_index: u32) -> PathBuf {
        self.directory
            .join(format!("target-{target_index}.authorization"))
    }
}

/// Where OSflux keeps its state: `OSFLUX_STATE_DIR` when set, otherwise the
/// per-user application data folder for the platform.
pub fn state_directory() -> Option<PathBuf> {
    if let Some(directory) = non_empty_env(STATE_DIR_ENV) {
        return Some(PathBuf::from(directory));
    }

    if cfg!(windows) {
        non_empty_env("LOCALAPPDATA").map(|directory| PathBuf::from(directory).join("OSflux"))
    } else {
        non_empty_env("XDG_STATE_HOME")
            .map(|directory| PathBuf::from(directory).join("osflux"))
            .or_else(|| {
                non_empty_env("HOME").map(|home| {
                    PathBuf::from(home)
                        .join(".local")
                        .join("state")
                        .join("osflux")
                })
            })
    }
}

fn non_empty_env(name: &str) -> Option<std::ffi::OsString> {
    env::var_os(name).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        record_write_authorization, AuthorizationProblem, AuthorizationStore,
        AuthorizationStoreError, ImageIdentity, WriteAuthorization, AUTHORIZATION_LIFETIME_SECONDS,
    };
    use crate::preflight::WritePreflightStatus;
    use crate::write_intent::{ConfirmationForm, WriteIntent, WriteIntentStatus};
    use std::fs;
    use std::path::PathBuf;

    const NOW: u64 = 1_795_022_016;

    #[test]
    fn issues_an_authorization_for_a_confirmable_intent() {
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();

        assert_eq!(authorization.target_index, 3);
        assert_eq!(authorization.target_identity_code, "2E85");
        assert_eq!(
            authorization.confirmation_phrase,
            "OSFLUX ERASE TARGET 3 2E85"
        );
        assert_eq!(
            authorization.expires_at_unix_seconds,
            NOW + AUTHORIZATION_LIFETIME_SECONDS
        );
    }

    #[test]
    fn issues_nothing_for_a_blocked_intent() {
        let mut blocked = intent();
        blocked.status = WriteIntentStatus::Blocked;
        blocked.confirmation_phrase = None;

        assert_eq!(WriteAuthorization::issue(&blocked, image(), NOW), None);
    }

    #[test]
    fn accepts_the_same_drive_image_and_phrase_within_the_lifetime() {
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();

        assert_eq!(
            check(&authorization, NOW + AUTHORIZATION_LIFETIME_SECONDS - 1),
            Ok(())
        );
    }

    #[test]
    fn refuses_after_the_lifetime() {
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();

        assert_eq!(
            check(&authorization, NOW + AUTHORIZATION_LIFETIME_SECONDS),
            Err(AuthorizationProblem::Expired {
                expired_at_unix_seconds: NOW + AUTHORIZATION_LIFETIME_SECONDS
            })
        );
    }

    #[test]
    fn refuses_an_authorization_dated_in_the_future() {
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW + 3600).unwrap();

        assert_eq!(
            check(&authorization, NOW),
            Err(AuthorizationProblem::IssuedInFuture)
        );
    }

    #[test]
    fn refuses_a_different_drive_phrase_or_image() {
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();
        let mut changed_image = image();
        changed_image.modified_unix_seconds += 1;

        assert_eq!(
            authorization.check(
                3,
                r"\\.\PhysicalDrive3",
                "0B21",
                &image(),
                "OSFLUX ERASE TARGET 3 2E85",
                NOW
            ),
            Err(AuthorizationProblem::TargetChanged)
        );
        assert_eq!(
            authorization.check(
                3,
                r"\\.\PhysicalDrive3",
                "2E85",
                &image(),
                "OSFLUX ERASE TARGET 3 XXXX",
                NOW
            ),
            Err(AuthorizationProblem::PhraseMismatch)
        );
        assert_eq!(
            authorization.check(
                3,
                r"\\.\PhysicalDrive3",
                "2E85",
                &changed_image,
                "OSFLUX ERASE TARGET 3 2E85",
                NOW
            ),
            Err(AuthorizationProblem::ImageMismatch)
        );
    }

    #[test]
    fn accepts_the_identity_code_when_the_drive_was_empty() {
        let mut empty_drive = intent();
        empty_drive.short_confirmation = Some("2E85".to_string());
        let authorization = WriteAuthorization::issue(&empty_drive, image(), NOW).unwrap();

        assert_eq!(
            check_with(&authorization, "2e85", NOW),
            Ok(ConfirmationForm::IdentityCode)
        );
        assert_eq!(
            check_with(&authorization, "OSFLUX ERASE TARGET 3 2E85", NOW),
            Ok(ConfirmationForm::FullPhrase)
        );
    }

    #[test]
    fn refuses_the_identity_code_when_the_drive_was_not_empty() {
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();

        assert_eq!(
            check_with(&authorization, "2E85", NOW),
            Err(AuthorizationProblem::ShortConfirmationNotRecorded)
        );
        assert_eq!(
            check_with(&authorization, "0B21", NOW),
            Err(AuthorizationProblem::PhraseMismatch)
        );
    }

    #[test]
    fn keeps_the_short_confirmation_across_save_and_load() {
        let store = AuthorizationStore::new(temp_dir());
        let mut empty_drive = intent();
        empty_drive.short_confirmation = Some("2E85".to_string());
        let with_code = WriteAuthorization::issue(&empty_drive, image(), NOW).unwrap();
        let without_code = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();

        store.save(&with_code).unwrap();
        let loaded_with = store.load(3).unwrap();
        store.save(&without_code).unwrap();
        let loaded_without = store.load(3).unwrap();
        let _ = fs::remove_dir_all(store.directory());

        assert_eq!(loaded_with, Some(with_code));
        assert_eq!(loaded_without, Some(without_code));
    }

    #[test]
    fn refuses_a_record_from_an_older_version() {
        let store = AuthorizationStore::new(temp_dir());
        let mut record = WriteAuthorization::issue(&intent(), image(), NOW)
            .unwrap()
            .to_record()
            .unwrap();
        record = record
            .replace(
                "osflux-write-authorization 2",
                "osflux-write-authorization 1",
            )
            .lines()
            .filter(|line| !line.starts_with("short_confirmation="))
            .collect::<Vec<_>>()
            .join("\n");
        fs::create_dir_all(store.directory()).unwrap();
        fs::write(store.directory().join("target-3.authorization"), record).unwrap();

        let loaded = store.load(3);
        let _ = fs::remove_dir_all(store.directory());

        assert!(matches!(
            loaded,
            Err(AuthorizationStoreError::Corrupt { .. })
        ));
    }

    #[test]
    fn saves_loads_and_consumes_once() {
        let store = AuthorizationStore::new(temp_dir());
        let authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();

        store.save(&authorization).unwrap();
        assert_eq!(store.load(3).unwrap(), Some(authorization));
        assert!(store.load(4).unwrap().is_none());

        store.consume(3).unwrap();
        assert!(store.load(3).unwrap().is_none());
        assert!(matches!(
            store.consume(3),
            Err(AuthorizationStoreError::Io { .. })
        ));

        let leftovers: Vec<_> = fs::read_dir(store.directory()).unwrap().collect();
        assert!(leftovers.is_empty(), "temporary files left behind");
        let _ = fs::remove_dir_all(store.directory());
    }

    #[test]
    fn a_newer_authorization_replaces_the_older_one() {
        let store = AuthorizationStore::new(temp_dir());
        let first = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();
        let mut other_image = image();
        other_image.path = r"C:\images\other.iso".to_string();
        let second = WriteAuthorization::issue(&intent(), other_image, NOW + 5).unwrap();

        store.save(&first).unwrap();
        store.save(&second).unwrap();

        assert_eq!(store.load(3).unwrap(), Some(second));
        let _ = fs::remove_dir_all(store.directory());
    }

    #[test]
    fn treats_an_unreadable_record_as_an_error_not_as_permission() {
        let store = AuthorizationStore::new(temp_dir());
        fs::create_dir_all(store.directory()).unwrap();
        fs::write(
            store.directory().join("target-3.authorization"),
            "osflux-write-authorization 2\ntarget_index=3\n",
        )
        .unwrap();

        assert!(matches!(
            store.load(3),
            Err(AuthorizationStoreError::Corrupt { .. })
        ));
        let _ = fs::remove_dir_all(store.directory());
    }

    #[test]
    fn refuses_to_record_a_value_with_a_line_break() {
        let store = AuthorizationStore::new(temp_dir());
        let mut authorization = WriteAuthorization::issue(&intent(), image(), NOW).unwrap();
        authorization.image.path = "C:\\images\nexpires_at_unix_seconds=99".to_string();

        assert!(matches!(
            store.save(&authorization),
            Err(AuthorizationStoreError::Unrecordable { .. })
        ));
        let _ = fs::remove_dir_all(store.directory());
    }

    #[test]
    fn records_an_authorization_for_a_ready_intent() {
        let directory = temp_dir();
        fs::create_dir_all(&directory).unwrap();
        let image_path = directory.join("alpine.iso");
        fs::write(&image_path, vec![0u8; 4096]).unwrap();
        let store = AuthorizationStore::new(directory.join("authorizations"));
        let mut ready = intent();

        let recorded = record_write_authorization(&mut ready, &image_path, &store, NOW)
            .unwrap()
            .unwrap();

        assert_eq!(store.load(3).unwrap(), Some(recorded));
        assert_eq!(
            ready.authorization_expires_at_unix_seconds,
            Some(NOW + AUTHORIZATION_LIFETIME_SECONDS)
        );
        assert!(ready
            .notes
            .iter()
            .any(|note| note.starts_with("Confirmation recorded for one write until")));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn records_nothing_for_a_blocked_intent() {
        let directory = temp_dir();
        let store = AuthorizationStore::new(directory.join("authorizations"));
        let mut blocked = intent();
        blocked.status = WriteIntentStatus::Blocked;

        let recorded =
            record_write_authorization(&mut blocked, "missing.iso", &store, NOW).unwrap();

        assert_eq!(recorded, None);
        assert!(!store.directory().exists());
    }

    #[test]
    fn reads_the_identity_of_a_real_file() {
        let directory = temp_dir();
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("disk.img");
        fs::write(&path, vec![0u8; 4096]).unwrap();

        let identity = ImageIdentity::read(&path).unwrap();
        let _ = fs::remove_dir_all(&directory);

        assert_eq!(identity.size_bytes, 4096);
        assert!(identity.path.ends_with("disk.img"));
    }

    fn check(authorization: &WriteAuthorization, now: u64) -> Result<(), AuthorizationProblem> {
        check_with(authorization, "OSFLUX ERASE TARGET 3 2E85", now).map(|_| ())
    }

    fn check_with(
        authorization: &WriteAuthorization,
        supplied: &str,
        now: u64,
    ) -> Result<ConfirmationForm, AuthorizationProblem> {
        authorization.check(3, r"\\.\PhysicalDrive3", "2E85", &image(), supplied, now)
    }

    fn intent() -> WriteIntent {
        WriteIntent {
            status: WriteIntentStatus::ConfirmationRequired,
            reason: "preflight passed".to_string(),
            confirmation_phrase: Some("OSFLUX ERASE TARGET 3 2E85".to_string()),
            short_confirmation: None,
            image_source: r"C:\images\alpine.iso".to_string(),
            target_index: 3,
            target_path: r"\\.\PhysicalDrive3".to_string(),
            target_model: Some("General UDisk USB Device".to_string()),
            target_size_bytes: Some(960 * 1024 * 1024),
            target_identity_code: Some("2E85".to_string()),
            preflight_status: WritePreflightStatus::NeedsConfirmation,
            preflight_reason: "passed".to_string(),
            authorization_expires_at_unix_seconds: None,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn image() -> ImageIdentity {
        ImageIdentity {
            path: r"C:\images\alpine.iso".to_string(),
            size_bytes: 370_147_328,
            modified_unix_seconds: NOW - 600,
        }
    }

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "osflux-authorization-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
