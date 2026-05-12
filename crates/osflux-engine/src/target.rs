use std::fmt;

use osflux_core::device::{self, DeviceAccess, PhysicalDevice};

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

    if likely_removable {
        notes.push("Device looks removable or external.".to_string());
    } else {
        warnings.push("Device is not clearly removable or external.".to_string());
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

    if device.contains_system_volume {
        warnings.push("Device contains the Windows system volume.".to_string());
    }

    let (status, reason, requires_confirmation) = classify_target(device, likely_removable);

    TargetAssessment {
        index: device.index,
        path: device.path.clone(),
        status,
        reason: reason.to_string(),
        size_bytes: device.size_bytes,
        model: device.model.clone(),
        interface_type: device.interface_type.clone(),
        media_type: device.media_type.clone(),
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
) -> (TargetAssessmentStatus, &'static str, bool) {
    if device.contains_system_volume {
        return (
            TargetAssessmentStatus::Blocked,
            "target contains the system volume",
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

    if !likely_removable {
        return (
            TargetAssessmentStatus::Blocked,
            "target is not clearly removable or external",
            false,
        );
    }

    (
        TargetAssessmentStatus::NeedsConfirmation,
        "target looks eligible but still requires explicit confirmation",
        true,
    )
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
        assert_eq!(
            assessment.reason,
            "target is not clearly removable or external"
        );
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
            pnp_device_id: None,
            contains_system_volume: system,
            access: DeviceAccess::Readable,
            note: None,
        }
    }
}
