use std::fmt;
use std::path::Path;

use crate::drive_contents::{DriveContents, DriveContentsState};
use crate::preflight::{
    run_write_preflight, WritePreflight, WritePreflightError, WritePreflightStatus,
};
use crate::size_label::size_label;
use crate::target::TargetAssessmentStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteIntent {
    pub status: WriteIntentStatus,
    pub reason: String,
    pub confirmation_phrase: Option<String>,
    /// The drive's identity code, offered as a short confirmation only when
    /// the drive is empty. The full phrase is always accepted as well.
    pub short_confirmation: Option<String>,
    /// For a drive with files on it whose partitions could be read: what
    /// will be lost, such as `Erase SMI USB DISK (14.8 GB) and the 1
    /// partition with files on it`. The identity code together with this
    /// exact acknowledgement also confirms the write. Studio offers it; the
    /// CLI asks for the full phrase.
    pub contents_acknowledgement: Option<String>,
    /// The drive's layout fingerprint when the acknowledgement was built, so
    /// the write can stop if the drive changes in between.
    pub contents_fingerprint: Option<String>,
    pub image_source: String,
    pub target_index: u32,
    pub target_path: String,
    pub target_model: Option<String>,
    pub target_size_bytes: Option<u64>,
    pub target_identity_code: Option<String>,
    pub preflight_status: WritePreflightStatus,
    pub preflight_reason: String,
    /// Set once a one-time confirmation has been recorded for this intent.
    pub authorization_expires_at_unix_seconds: Option<u64>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl WriteIntent {
    /// `short` when the identity code alone confirms the write, `full` when
    /// the whole phrase is needed, `None` when the intent cannot be confirmed.
    pub fn confirmation_level(&self) -> Option<&'static str> {
        match (&self.confirmation_phrase, &self.short_confirmation) {
            (None, _) => None,
            (Some(_), Some(_)) => Some("short"),
            (Some(_), None) => Some("full"),
        }
    }

    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"confirmationPhrase\": {},\n",
                "  \"confirmationLevel\": {},\n",
                "  \"shortConfirmation\": {},\n",
                "  \"contentsAcknowledgement\": {},\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"targetIndex\": {},\n",
                "  \"targetPath\": \"{}\",\n",
                "  \"targetModel\": {},\n",
                "  \"targetSizeBytes\": {},\n",
                "  \"targetIdentityCode\": {},\n",
                "  \"preflightStatus\": \"{}\",\n",
                "  \"preflightReason\": \"{}\",\n",
                "  \"authorizationExpiresAtUnixSeconds\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            escape_json_string(&self.reason),
            optional_string_json(self.confirmation_phrase.as_deref()),
            optional_string_json(self.confirmation_level()),
            optional_string_json(self.short_confirmation.as_deref()),
            optional_string_json(self.contents_acknowledgement.as_deref()),
            escape_json_string(&self.image_source),
            self.target_index,
            escape_json_string(&self.target_path),
            optional_string_json(self.target_model.as_deref()),
            optional_number_json(self.target_size_bytes),
            optional_string_json(self.target_identity_code.as_deref()),
            self.preflight_status.as_str(),
            escape_json_string(&self.preflight_reason),
            optional_number_json(self.authorization_expires_at_unix_seconds),
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

/// The phrase an operator types to confirm erasing a target drive.
pub fn confirmation_phrase(target_index: u32, identity_code: &str) -> String {
    format!("OSFLUX ERASE TARGET {target_index} {identity_code}")
}

/// Which of the accepted confirmations was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationForm {
    /// `OSFLUX ERASE TARGET <index> <code>`, accepted for any drive.
    FullPhrase,
    /// The four-character identity code alone, accepted only for an empty
    /// drive.
    IdentityCode,
    /// The identity code together with the acknowledgement that names what
    /// will be lost, accepted only for a drive whose contents could be read
    /// and only while they stay as they were.
    CodeWithContentsAcknowledged,
}

impl ConfirmationForm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FullPhrase => "full-phrase",
            Self::IdentityCode => "identity-code",
            Self::CodeWithContentsAcknowledged => "identity-code-contents-acknowledged",
        }
    }
}

/// What the person supplied to confirm a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppliedConfirmation<'a> {
    /// Typed text: the full phrase, or the identity code for an empty drive.
    Typed(&'a str),
    /// The identity code, and the acknowledgement exactly as it was shown
    /// next to the box that was ticked.
    CodeWithContentsAcknowledged {
        code: &'a str,
        acknowledgement: &'a str,
    },
}

impl<'a> From<&'a str> for SuppliedConfirmation<'a> {
    fn from(text: &'a str) -> Self {
        Self::Typed(text)
    }
}

impl<'a> From<&'a String> for SuppliedConfirmation<'a> {
    fn from(text: &'a String) -> Self {
        Self::Typed(text)
    }
}

impl SuppliedConfirmation<'_> {
    /// The identity code or text that was typed.
    pub fn typed(&self) -> &str {
        match self {
            Self::Typed(text) => text,
            Self::CodeWithContentsAcknowledged { code, .. } => code,
        }
    }
}

/// The acknowledgement for a drive with files on it, naming what will be
/// lost: `Erase SMI USB DISK (14.8 GB) and the 1 partition with files on
/// it`. `None` unless the drive's partitions could be read, since the
/// acknowledgement must say what it covers.
pub fn contents_acknowledgement(
    model: Option<&str>,
    size_bytes: Option<u64>,
    contents: &DriveContents,
) -> Option<String> {
    if contents.state != DriveContentsState::HasData || contents.partitions.is_empty() {
        return None;
    }
    let model = model.map(str::trim).filter(|model| !model.is_empty());
    let drive = match (model, size_bytes) {
        (Some(model), Some(size)) => format!("{model} ({})", size_label(size)),
        (Some(model), None) => model.to_string(),
        (None, Some(size)) => format!("this drive ({})", size_label(size)),
        (None, None) => "this drive".to_string(),
    };
    let partitions = match contents.partitions.len() {
        1 => "the 1 partition".to_string(),
        count => format!("the {count} partitions"),
    };
    Some(format!("Erase {drive} and {partitions} with files on it"))
}

/// Matches a supplied confirmation. Typed text is matched as
/// [`match_confirmation`] does. The identity code with an acknowledgement
/// matches only when an acknowledgement is on offer, the code is this
/// drive's (ignoring surrounding spaces and letter case), and the
/// acknowledgement is exactly the one offered.
pub fn match_supplied_confirmation(
    full_phrase: Option<&str>,
    short_code: Option<&str>,
    identity_code: Option<&str>,
    offered_acknowledgement: Option<&str>,
    supplied: &SuppliedConfirmation<'_>,
) -> Option<ConfirmationForm> {
    match supplied {
        SuppliedConfirmation::Typed(text) => match_confirmation(full_phrase, short_code, text),
        SuppliedConfirmation::CodeWithContentsAcknowledged {
            code,
            acknowledgement,
        } => {
            let code_matches =
                identity_code.is_some_and(|identity| code.trim().eq_ignore_ascii_case(identity));
            let acknowledged = offered_acknowledgement == Some(*acknowledgement);
            (code_matches && acknowledged).then_some(ConfirmationForm::CodeWithContentsAcknowledged)
        }
    }
}

/// Matches what the operator typed against the full phrase and, when one is
/// on offer, the short identity code. The phrase must match exactly; the code
/// ignores surrounding spaces and letter case.
pub fn match_confirmation(
    full_phrase: Option<&str>,
    short_code: Option<&str>,
    supplied: &str,
) -> Option<ConfirmationForm> {
    if full_phrase == Some(supplied) {
        return Some(ConfirmationForm::FullPhrase);
    }
    short_code
        .filter(|code| supplied.trim().eq_ignore_ascii_case(code))
        .map(|_| ConfirmationForm::IdentityCode)
}

pub fn build_write_intent(preflight: &WritePreflight) -> WriteIntent {
    let status = classify_intent_status(preflight.status);
    // The identity code ties the phrase to this drive, so a phrase confirmed
    // for one drive stops matching if a different drive takes the same index.
    let confirmation_phrase = match (status, preflight.target_identity_code.as_deref()) {
        (WriteIntentStatus::ConfirmationRequired, Some(identity_code)) => {
            Some(confirmation_phrase(preflight.target_index, identity_code))
        }
        _ => None,
    };
    // Only an empty drive gets the short confirmation: there is nothing on it
    // that a mistake could cost.
    let short_confirmation = match (
        &confirmation_phrase,
        preflight.target_status,
        preflight.target_identity_code.as_deref(),
    ) {
        (Some(_), TargetAssessmentStatus::Eligible, Some(identity_code)) => {
            Some(identity_code.to_string())
        }
        _ => None,
    };
    // A drive with files gets the acknowledgement instead, when what it
    // holds could be read and named.
    let contents_acknowledgement = match (&confirmation_phrase, &short_confirmation) {
        (Some(_), None) => preflight.target_contents.as_ref().and_then(|contents| {
            contents_acknowledgement(
                preflight.target_model.as_deref(),
                preflight.target_size_bytes,
                contents,
            )
        }),
        _ => None,
    };
    let contents_fingerprint = contents_acknowledgement.as_ref().and_then(|_| {
        preflight
            .target_contents
            .as_ref()
            .map(DriveContents::layout_fingerprint)
    });

    let mut notes = Vec::new();
    notes.push("Write intent is read-only; no device writes were attempted.".to_string());
    notes.push(
        "--write --execute needs this exact phrase, and the confirmation works for one write within 10 minutes."
            .to_string(),
    );
    notes.push(
        "The phrase includes the target's identity code, so it stops matching if a different drive takes this index."
            .to_string(),
    );
    if let Some(code) = &short_confirmation {
        notes.push(format!(
            "The drive is empty, so typing its identity code {code} also confirms the write. If anything is added to the drive first, the code is refused."
        ));
    }

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
        short_confirmation,
        contents_acknowledgement,
        contents_fingerprint,
        image_source: preflight.image_source.clone(),
        target_index: preflight.target_index,
        target_path: preflight.target_path.clone(),
        target_model: preflight.target_model.clone(),
        target_size_bytes: preflight.target_size_bytes,
        target_identity_code: preflight.target_identity_code.clone(),
        preflight_status: preflight.status,
        preflight_reason: preflight.reason.clone(),
        authorization_expires_at_unix_seconds: None,
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
    use super::{
        build_write_intent, contents_acknowledgement, match_confirmation,
        match_supplied_confirmation, ConfirmationForm, SuppliedConfirmation, WriteIntentStatus,
    };
    use crate::drive_contents::{
        DriveContents, DriveContentsState, DrivePartition, FilesystemKind, PartitionScheme,
    };
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
    fn offers_the_identity_code_for_an_empty_drive() {
        let mut empty_drive = preflight(WritePreflightStatus::Ready);
        empty_drive.target_status = TargetAssessmentStatus::Eligible;

        let intent = build_write_intent(&empty_drive);

        assert_eq!(intent.short_confirmation.as_deref(), Some("7F3A"));
        assert_eq!(intent.confirmation_level(), Some("short"));
        assert_eq!(
            intent.confirmation_phrase.as_deref(),
            Some("OSFLUX ERASE TARGET 2 7F3A")
        );
        let json = intent.to_json_pretty();
        assert!(json.contains("\"confirmationLevel\": \"short\""), "{json}");
        assert!(json.contains("\"shortConfirmation\": \"7F3A\""), "{json}");
    }

    #[test]
    fn needs_the_full_phrase_for_a_drive_with_data() {
        let intent = build_write_intent(&preflight(WritePreflightStatus::NeedsConfirmation));

        assert_eq!(intent.short_confirmation, None);
        assert_eq!(intent.confirmation_level(), Some("full"));
    }

    #[test]
    fn offers_nothing_for_a_blocked_drive() {
        let mut blocked = preflight(WritePreflightStatus::Blocked);
        blocked.target_status = TargetAssessmentStatus::Eligible;

        let intent = build_write_intent(&blocked);

        assert_eq!(intent.short_confirmation, None);
        assert_eq!(intent.confirmation_level(), None);
    }

    #[test]
    fn matches_the_phrase_exactly_and_the_code_loosely() {
        let phrase = Some("OSFLUX ERASE TARGET 2 7F3A");

        assert_eq!(
            match_confirmation(phrase, Some("7F3A"), "OSFLUX ERASE TARGET 2 7F3A"),
            Some(ConfirmationForm::FullPhrase)
        );
        assert_eq!(
            match_confirmation(phrase, None, "OSFLUX ERASE TARGET 2 7F3A"),
            Some(ConfirmationForm::FullPhrase)
        );
        assert_eq!(
            match_confirmation(phrase, Some("7F3A"), " 7f3a "),
            Some(ConfirmationForm::IdentityCode)
        );
        assert_eq!(match_confirmation(phrase, None, "7F3A"), None);
        assert_eq!(match_confirmation(phrase, Some("7F3A"), "7F3B"), None);
        assert_eq!(
            match_confirmation(phrase, Some("7F3A"), "osflux erase target 2 7f3a"),
            None
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
            target_contents: None,
            requires_confirmation: true,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn contents(state: DriveContentsState, partitions: usize) -> DriveContents {
        DriveContents {
            state,
            reason: state.as_str().to_string(),
            partition_scheme: Some(PartitionScheme::Mbr),
            partitions: (1..=partitions as u32)
                .map(|number| DrivePartition {
                    number,
                    start_bytes: u64::from(number) * 1024 * 1024,
                    size_bytes: 8 * 1024 * 1024,
                    filesystem: Some(FilesystemKind::Fat32),
                    label: Some("FILES".to_string()),
                    used_bytes: Some(4096),
                })
                .collect(),
        }
    }

    const ACKNOWLEDGEMENT: &str =
        "Erase SMI USB DISK (14.8 GB) and the 1 partition with files on it";

    #[test]
    fn names_what_will_be_lost_for_a_drive_with_files() {
        let mut preflight = preflight(WritePreflightStatus::NeedsConfirmation);
        preflight.target_model = Some("SMI USB DISK".to_string());
        preflight.target_size_bytes = Some(15_931_539_456);
        preflight.target_contents = Some(contents(DriveContentsState::HasData, 1));

        let intent = build_write_intent(&preflight);

        assert_eq!(
            intent.contents_acknowledgement.as_deref(),
            Some(ACKNOWLEDGEMENT)
        );
        assert_eq!(
            intent.contents_fingerprint,
            preflight
                .target_contents
                .as_ref()
                .map(DriveContents::layout_fingerprint)
        );
        // The full phrase stays on offer, for the CLI.
        assert_eq!(intent.confirmation_level(), Some("full"));
        assert!(intent.to_json_pretty().contains(&format!(
            "\"contentsAcknowledgement\": \"{ACKNOWLEDGEMENT}\""
        )));
    }

    #[test]
    fn counts_every_partition_in_the_acknowledgement() {
        let text = contents_acknowledgement(
            Some("Generic Flash"),
            None,
            &contents(DriveContentsState::HasData, 2),
        );
        assert_eq!(
            text.as_deref(),
            Some("Erase Generic Flash and the 2 partitions with files on it")
        );
        let unnamed = contents_acknowledgement(
            None,
            Some(1 << 30),
            &contents(DriveContentsState::HasData, 1),
        );
        assert_eq!(
            unnamed.as_deref(),
            Some("Erase this drive (1.00 GB) and the 1 partition with files on it")
        );
    }

    #[test]
    fn offers_no_acknowledgement_without_readable_partitions() {
        // Unreadable, or data with no partition to name: the full phrase is
        // the only way to confirm.
        for contents in [
            contents(DriveContentsState::Unknown, 0),
            contents(DriveContentsState::HasData, 0),
        ] {
            let mut preflight = preflight(WritePreflightStatus::NeedsConfirmation);
            preflight.target_contents = Some(contents);
            let intent = build_write_intent(&preflight);
            assert_eq!(intent.contents_acknowledgement, None);
            assert_eq!(intent.contents_fingerprint, None);
        }
        let intent = build_write_intent(&preflight(WritePreflightStatus::NeedsConfirmation));
        assert_eq!(intent.contents_acknowledgement, None);
    }

    #[test]
    fn an_empty_drive_gets_the_short_code_not_the_acknowledgement() {
        let mut preflight = preflight(WritePreflightStatus::Ready);
        preflight.target_status = TargetAssessmentStatus::Eligible;
        preflight.target_contents = Some(contents(DriveContentsState::EmptyFilesystem, 1));

        let intent = build_write_intent(&preflight);

        assert_eq!(intent.short_confirmation.as_deref(), Some("7F3A"));
        assert_eq!(intent.contents_acknowledgement, None);
    }

    #[test]
    fn a_blocked_intent_offers_no_acknowledgement() {
        let mut preflight = preflight(WritePreflightStatus::Blocked);
        preflight.target_contents = Some(contents(DriveContentsState::HasData, 1));

        assert_eq!(
            build_write_intent(&preflight).contents_acknowledgement,
            None
        );
    }

    #[test]
    fn matches_the_code_only_with_the_exact_acknowledgement() {
        let phrase = "OSFLUX ERASE TARGET 2 7F3A";
        let check = |code: &str, acknowledgement: &str, offer: Option<&str>| {
            match_supplied_confirmation(
                Some(phrase),
                None,
                Some("7F3A"),
                offer,
                &SuppliedConfirmation::CodeWithContentsAcknowledged {
                    code,
                    acknowledgement,
                },
            )
        };

        assert_eq!(
            check(" 7f3a ", ACKNOWLEDGEMENT, Some(ACKNOWLEDGEMENT)),
            Some(ConfirmationForm::CodeWithContentsAcknowledged)
        );
        assert_eq!(check("7F3B", ACKNOWLEDGEMENT, Some(ACKNOWLEDGEMENT)), None);
        assert_eq!(
            check("7F3A", "Erase SMI USB DISK", Some(ACKNOWLEDGEMENT)),
            None
        );
        assert_eq!(check("7F3A", ACKNOWLEDGEMENT, None), None);
        // The code typed on its own is not enough for a drive with files.
        let typed = |text| {
            match_supplied_confirmation(
                Some(phrase),
                None,
                Some("7F3A"),
                Some(ACKNOWLEDGEMENT),
                &SuppliedConfirmation::Typed(text),
            )
        };
        assert_eq!(typed("7F3A"), None);
        assert_eq!(typed(phrase), Some(ConfirmationForm::FullPhrase));
    }
}
