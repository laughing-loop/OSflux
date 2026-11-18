use std::fmt;

use osflux_core::device::{self, DeviceAccess, PhysicalDevice};

use crate::fingerprint::stable_fingerprint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetAssessment {
    pub index: u32,
    pub path: String,
    pub status: TargetAssessmentStatus,
    pub reason: String,
    pub size_bytes: Option<u64>,
    pub model: Option<String>,
    pub interface_type: Option<String>,
    pub media_type: Option<String>,
    pub serial_number: Option<String>,
    pub identity_code: Option<String>,
    pub access: DeviceAccess,
    pub contains_system_volume: bool,
    pub likely_removable: bool,
    pub requires_confirmation: bool,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl TargetAssessment {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"index\": {},\n",
                "  \"path\": \"{}\",\n",
                "  \"status\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"sizeBytes\": {},\n",
                "  \"model\": {},\n",
                "  \"interfaceType\": {},\n",
                "  \"mediaType\": {},\n",
                "  \"serialNumber\": {},\n",
                "  \"identityCode\": {},\n",
                "  \"access\": \"{}\",\n",
                "  \"containsSystemVolume\": {},\n",
                "  \"likelyRemovable\": {},\n",
                "  \"requiresConfirmation\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.index,
            escape_json_string(&self.path),
            self.status.as_str(),
            escape_json_string(&self.reason),
            optional_number_json(self.size_bytes),
            optional_string_json(self.model.as_deref()),
            optional_string_json(self.interface_type.as_deref()),
            optional_string_json(self.media_type.as_deref()),
            optional_string_json(self.serial_number.as_deref()),
            optional_string_json(self.identity_code.as_deref()),
            self.access.as_str(),
            self.contains_system_volume,
            self.likely_removable,
            self.requires_confirmation,
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetAssessmentStatus {
    Eligible,
    NeedsConfirmation,
    Blocked,
}

impl TargetAssessmentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::NeedsConfirmation => "needs-confirmation",
            Self::Blocked => "blocked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetAssessmentError {
    NotFound { index: u32 },
}

impl fmt::Display for TargetAssessmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { index } => {
                write!(formatter, "target device index was not found: {index}")
            }
        }
    }
}

impl std::error::Error for TargetAssessmentError {}

pub fn assess_target(index: u32) -> Result<TargetAssessment, TargetAssessmentError> {
    let Some(device) = device::list_physical_devices()
        .into_iter()
        .find(|device| device.index == index)
    else {
        return Err(TargetAssessmentError::NotFound { index });
    };

    Ok(assess_target_device(&device))
}

pub fn assess_target_device(device: &PhysicalDevice) -> TargetAssessment {
    let mut notes = Vec::new();
    let mut warnings = Vec::new();
    let likely_removable = device.is_likely_removable_target();
    let identity_code = target_identity_code(device);

    if device.is_external_disk() {
        warnings.push(
            "Device is an external hard disk or SSD; OSflux only writes to removable media such as USB sticks and SD cards."
                .to_string(),
        );
    } else if likely_removable {
        notes.push("Device reports removable media.".to_string());
    } else {
        warnings.push("Device does not report removable media.".to_string());
    }

    if device.access == DeviceAccess::MetadataOnly {
        warnings.push(
            "Only metadata access is available; run as Administrator before any write workflow."
                .to_string(),
        );
    }

    if device.size_bytes.is_none() {
        warnings.push("Device size is unknown.".to_string());
    }

    if identity_code.is_none() {
        warnings.push("Device identity could not be read.".to_string());
    }

    if device.contains_system_volume {
        warnings.push("Device contains the Windows system volume.".to_string());
    }

    let (status, reason, requires_confirmation) =
        classify_target(device, likely_removable, identity_code.is_some());

    TargetAssessment {
        index: device.index,
        path: device.path.clone(),
        status,
        reason: reason.to_string(),
        size_bytes: device.size_bytes,
        model: device.model.clone(),
        interface_type: device.interface_type.clone(),
        media_type: device.media_type.clone(),
        serial_number: device.serial_number.clone(),
        identity_code,
        access: device.access,
        contains_system_volume: device.contains_system_volume,
        likely_removable,
        requires_confirmation,
        notes,
        warnings,
    }
}

fn classify_target(
    device: &PhysicalDevice,
    likely_removable: bool,
    has_identity: bool,
) -> (TargetAssessmentStatus, &'static str, bool) {
    if device.contains_system_volume {
        return (
            TargetAssessmentStatus::Blocked,
            "target contains the system volume",
            false,
        );
    }

    if device.is_external_disk() {
        return (
            TargetAssessmentStatus::Blocked,
            "target is an external hard disk or SSD",
            false,
        );
    }

    if device.size_bytes.is_none() {
        return (
            TargetAssessmentStatus::Blocked,
            "target size is unknown",
            false,
        );
    }

    if !has_identity {
        return (
            TargetAssessmentStatus::Blocked,
            "target identity could not be read",
            false,
        );
    }

    if !likely_removable {
        return (
            TargetAssessmentStatus::Blocked,
            "target does not report removable media",
            false,
        );
    }

    (
        TargetAssessmentStatus::NeedsConfirmation,
        "target looks eligible but still requires explicit confirmation",
        true,
    )
}

/// Short code that changes when a different drive takes the same index.
/// Size is left out on purpose: it comes from the drive itself with
/// administrator rights but from Windows' drive inventory without them, and
/// the two values differ, so including it would change the code between runs.
fn target_identity_code(device: &PhysicalDevice) -> Option<String> {
    let fields = [
        device.model.as_deref(),
        device.serial_number.as_deref(),
        device.pnp_device_id.as_deref(),
    ];

    if fields.iter().all(Option::is_none) {
        return None;
    }

    let seed = fields.map(|field| field.unwrap_or("")).join("|");
    Some(format!(
        "{:04X}",
        stable_fingerprint(seed.as_bytes()) & 0xFFFF
    ))
}

fn optional_number_json(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn optional_string_json(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", escape_json_string(value)))
        .unwrap_or_else(|| "null".to_string())
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
    use super::{assess_target_device, TargetAssessmentStatus};
    use osflux_core::device::{DeviceAccess, PhysicalDevice};

    #[test]
    fn blocks_system_drive() {
        let assessment = assess_target_device(&device(true, true, Some(32 * 1024 * 1024)));

        assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);
        assert_eq!(assessment.reason, "target contains the system volume");
        assert!(!assessment.requires_confirmation);
    }

    #[test]
    fn blocks_internal_or_unknown_drive() {
        let mut target = device(false, false, Some(32 * 1024 * 1024));
        target.interface_type = Some("SCSI".to_string());

        let assessment = assess_target_device(&target);

        assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);
        assert_eq!(assessment.reason, "target does not report removable media");
    }

    #[test]
    fn blocks_external_ssd_connected_over_usb() {
        let mut target = device(false, true, Some(238 * 1024 * 1024 * 1024));
        target.model = Some("SAMSUNG SSD PM830 2.5\" 7 USB Device".to_string());
        target.media_type = Some("External hard disk media".to_string());

        let assessment = assess_target_device(&target);

        assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);
        assert_eq!(assessment.reason, "target is an external hard disk or SSD");
        assert!(!assessment.likely_removable);
        assert!(!assessment.requires_confirmation);
    }

    #[test]
    fn blocks_usb_drive_that_does_not_report_removable_media() {
        let mut target = device(false, true, Some(32 * 1024 * 1024));
        target.media_type = None;

        let assessment = assess_target_device(&target);

        assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);
        assert_eq!(assessment.reason, "target does not report removable media");
    }

    #[test]
    fn marks_removable_drive_as_needing_confirmation() {
        let assessment = assess_target_device(&device(false, true, Some(32 * 1024 * 1024)));

        assert_eq!(assessment.status, TargetAssessmentStatus::NeedsConfirmation);
        assert!(assessment.requires_confirmation);
        assert!(assessment.likely_removable);
    }

    #[test]
    fn serializes_assessment_as_json() {
        let assessment = assess_target_device(&device(false, true, Some(32 * 1024 * 1024)));
        let json = assessment.to_json_pretty();

        assert!(json.contains("\"status\": \"needs-confirmation\""));
        assert!(json.contains("\"path\": \"\\\\\\\\.\\\\PhysicalDrive2\""));
        assert!(json.contains("\"requiresConfirmation\": true"));
    }

    #[test]
    fn identity_code_changes_when_a_different_drive_takes_the_index() {
        let first = assess_target_device(&device(false, true, Some(32 * 1024 * 1024)));
        let mut other = device(false, true, Some(32 * 1024 * 1024));
        other.serial_number = Some("SN-0002".to_string());
        let second = assess_target_device(&other);

        let first_code = first.identity_code.unwrap();
        assert_eq!(first_code.len(), 4);
        assert_ne!(Some(first_code), second.identity_code);
    }

    #[test]
    fn identity_code_ignores_reported_size() {
        let without_admin = assess_target_device(&device(false, true, Some(32 * 1024 * 1024)));
        let with_admin = assess_target_device(&device(false, true, Some(33 * 1024 * 1024)));

        assert!(without_admin.identity_code.is_some());
        assert_eq!(without_admin.identity_code, with_admin.identity_code);
    }

    #[test]
    fn blocks_drive_without_readable_identity() {
        let mut target = device(false, true, Some(32 * 1024 * 1024));
        target.model = None;
        target.serial_number = None;
        target.pnp_device_id = None;

        let assessment = assess_target_device(&target);

        assert_eq!(assessment.status, TargetAssessmentStatus::Blocked);
        assert_eq!(assessment.reason, "target identity could not be read");
        assert_eq!(assessment.identity_code, None);
    }

    fn device(system: bool, removable: bool, size_bytes: Option<u64>) -> PhysicalDevice {
        PhysicalDevice {
            index: 2,
            path: "\\\\.\\PhysicalDrive2".to_string(),
            size_bytes,
            model: Some("Test Drive".to_string()),
            interface_type: Some(if removable { "USB" } else { "NVMe" }.to_string()),
            media_type: Some(
                if removable {
                    "Removable Media"
                } else {
                    "Fixed hard disk"
                }
                .to_string(),
            ),
            pnp_device_id: Some("USBSTOR\\DISK&VEN_TEST&PROD_DRIVE\\0001".to_string()),
            serial_number: Some("SN-0001".to_string()),
            contains_system_volume: system,
            access: DeviceAccess::Readable,
            note: None,
        }
    }
}
