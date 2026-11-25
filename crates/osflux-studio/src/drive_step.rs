//! Step 2 (frames 05–06): the drives the engine allows, and the ones it
//! will not erase with its reason in plain words. Whether a drive is allowed
//! is the engine's call; this module only words it and keeps the choice.

use osflux_engine::{DriveContentsState, TargetAssessment, TargetAssessmentStatus};

use crate::format::size_short;
use crate::{DriveKind, Status};

/// A drive that can be picked.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveEntry {
    pub index: u32,
    pub identity: Option<String>,
    pub model: String,
    pub path: String,
    pub size: String,
    pub bus: String,
    pub media: String,
    pub kind: DriveKind,
    pub status: Status,
    pub status_label: &'static str,
    pub status_detail: &'static str,
}

/// A drive OSflux will not erase.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockedEntry {
    pub model: String,
    pub path: String,
    pub size: String,
    pub bus: String,
    pub reason: String,
    pub kind: DriveKind,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DriveList {
    pub allowed: Vec<DriveEntry>,
    pub blocked: Vec<BlockedEntry>,
}

/// The drive the person picked: its position and the identity code it had
/// then, so a different drive in the same position is not mistaken for it.
#[derive(Debug, Clone, PartialEq)]
pub struct ChosenDrive {
    pub index: u32,
    pub identity: Option<String>,
    pub model: String,
}

impl DriveList {
    pub fn from_assessments(assessments: &[TargetAssessment]) -> Self {
        let mut list = DriveList::default();
        for assessment in assessments {
            match assessment.status {
                TargetAssessmentStatus::Blocked => list.blocked.push(BlockedEntry {
                    model: model_name(assessment),
                    path: assessment.path.clone(),
                    size: size_label(assessment.size_bytes),
                    bus: bus_label(assessment.interface_type.as_deref()),
                    reason: blocked_reason(&assessment.reason),
                    kind: if looks_like_sd_card(assessment) {
                        DriveKind::SdCard
                    } else {
                        DriveKind::Disk
                    },
                }),
                TargetAssessmentStatus::Eligible | TargetAssessmentStatus::NeedsConfirmation => {
                    let kind = if looks_like_sd_card(assessment) {
                        DriveKind::SdCard
                    } else {
                        DriveKind::Usb
                    };
                    let (status, status_label, status_detail) =
                        if assessment.status == TargetAssessmentStatus::Eligible {
                            (Status::Ready, "Ready", "")
                        } else {
                            (
                                Status::NeedsConfirmation,
                                "Needs confirmation",
                                contents_detail(assessment),
                            )
                        };
                    list.allowed.push(DriveEntry {
                        index: assessment.index,
                        identity: assessment.identity_code.clone(),
                        model: model_name(assessment),
                        path: assessment.path.clone(),
                        size: size_label(assessment.size_bytes),
                        bus: bus_label(assessment.interface_type.as_deref()),
                        media: media_label(assessment.media_type.as_deref(), kind),
                        kind,
                        status,
                        status_label,
                        status_detail,
                    })
                }
            }
        }
        list
    }

    /// The position in `allowed` of the chosen drive, if it is still there
    /// with the same identity.
    pub fn position_of(&self, chosen: &ChosenDrive) -> Option<usize> {
        self.allowed
            .iter()
            .position(|drive| drive.index == chosen.index && drive.identity == chosen.identity)
    }
}

impl DriveEntry {
    pub fn chosen(&self) -> ChosenDrive {
        ChosenDrive {
            index: self.index,
            identity: self.identity.clone(),
            model: self.model.clone(),
        }
    }
}

pub fn model_name(assessment: &TargetAssessment) -> String {
    assessment
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Drive {}", assessment.index))
}

fn size_label(size: Option<u64>) -> String {
    size.map(size_short)
        .unwrap_or_else(|| "Unknown".to_string())
}

/// Windows reports the bus as `USB`, `SCSI`, `IDE` and so on.
pub fn bus_label(interface: Option<&str>) -> String {
    match interface.map(str::trim) {
        None | Some("") => "–".to_string(),
        Some(bus) if bus.eq_ignore_ascii_case("nvme") => "NVMe".to_string(),
        Some(bus) => bus.to_ascii_uppercase(),
    }
}

/// Windows' media types in the words the design uses.
pub fn media_label(media: Option<&str>, kind: DriveKind) -> String {
    if kind == DriveKind::SdCard {
        return "SD card".to_string();
    }
    let Some(media) = media.map(str::trim).filter(|media| !media.is_empty()) else {
        return "Unknown".to_string();
    };
    let lower = media.to_ascii_lowercase();
    if lower.contains("removable") {
        "Removable media".to_string()
    } else if lower.contains("external") {
        "External disk".to_string()
    } else if lower.contains("fixed") {
        "Fixed disk".to_string()
    } else {
        let mut chars = lower.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => "Unknown".to_string(),
        }
    }
}

/// Card readers name themselves in the model, such as "SD/MMC" or "Card
/// Reader"; nothing else tells them apart from a USB stick.
fn looks_like_sd_card(assessment: &TargetAssessment) -> bool {
    let model = assessment
        .model
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    model.contains("sd/mmc")
        || model.contains("sd card")
        || model.contains("sdxc")
        || model.contains("sdhc")
        || model.contains("card reader")
        || model.contains("cardreader")
        || model
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word == "sd" || word == "mmc")
}

/// Why an allowed drive needs the full phrase.
fn contents_detail(assessment: &TargetAssessment) -> &'static str {
    match assessment.contents.as_ref().map(|contents| contents.state) {
        Some(DriveContentsState::HasData) => "Has files on it",
        Some(DriveContentsState::Blank | DriveContentsState::EmptyFilesystem) => "",
        Some(DriveContentsState::Unknown) | None => "Couldn't check what's on it",
    }
}

/// The engine's reasons for blocking a drive, in the design's words.
pub fn blocked_reason(reason: &str) -> String {
    match reason {
        "target contains the system volume" => {
            if cfg!(windows) {
                "System drive — Windows is installed here".to_string()
            } else {
                "System drive — this computer runs from it".to_string()
            }
        }
        "target is an external hard disk or SSD" => {
            "External SSD/HDD — usually holds data".to_string()
        }
        "target size is unknown" => "Size could not be read".to_string(),
        "target identity could not be read" => "Drive identity could not be read".to_string(),
        "target does not report removable media" => "Reports itself as a fixed disk".to_string(),
        other => {
            let other = other.strip_prefix("target ").unwrap_or(other);
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => "Blocked by the engine".to_string(),
            }
        }
    }
}

/// The action bar's hint on the Drive screen.
pub fn drive_hint(list: Option<&DriveList>, chosen: Option<&ChosenDrive>) -> String {
    match (list, chosen) {
        (_, Some(chosen)) => format!("Selected: {}", chosen.model),
        (Some(list), None) if list.allowed.is_empty() => {
            "Waiting for a USB stick or SD card.".to_string()
        }
        (Some(_), None) => "Choose a drive to continue.".to_string(),
        (None, None) => "Looking for drives…".to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{blocked_reason, bus_label, drive_hint, media_label, ChosenDrive, DriveList};
    use crate::{DriveKind, Status};
    use osflux_engine::{
        DriveContents, DriveContentsState, TargetAssessment, TargetAssessmentStatus,
    };

    pub(crate) fn assessment(
        index: u32,
        model: &str,
        size: u64,
        status: TargetAssessmentStatus,
        reason: &str,
    ) -> TargetAssessment {
        TargetAssessment {
            index,
            path: format!("\\\\.\\PhysicalDrive{index}"),
            status,
            reason: reason.to_string(),
            size_bytes: Some(size),
            model: Some(model.to_string()),
            interface_type: Some("USB".to_string()),
            media_type: Some("Removable Media".to_string()),
            serial_number: Some(format!("SERIAL{index}")),
            identity_code: Some(format!("{:04X}", 0x7F3A + index)),
            access: osflux_engine::DeviceAccess::MetadataOnly,
            contains_system_volume: false,
            likely_removable: true,
            requires_confirmation: true,
            contents: None,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn with_contents(
        mut assessment: TargetAssessment,
        state: DriveContentsState,
    ) -> TargetAssessment {
        assessment.contents = Some(DriveContents {
            state,
            reason: "test".to_string(),
            partition_scheme: None,
            partitions: Vec::new(),
        });
        assessment
    }

    #[test]
    fn the_list_matches_frame_05() {
        let assessments = vec![
            assessment(
                0,
                "Samsung SSD 980 1TB",
                1_000_204_886_016,
                TargetAssessmentStatus::Blocked,
                "target contains the system volume",
            ),
            assessment(
                1,
                "Samsung T7 Shield 2TB",
                2_000_398_934_016,
                TargetAssessmentStatus::Blocked,
                "target is an external hard disk or SSD",
            ),
            assessment(
                2,
                "SanDisk Ultra 64GB",
                64_023_511_040,
                TargetAssessmentStatus::Eligible,
                "target is empty",
            ),
            with_contents(
                assessment(
                    4,
                    "Generic SD/MMC USB Device",
                    15_931_539_456,
                    TargetAssessmentStatus::NeedsConfirmation,
                    "needs confirmation",
                ),
                DriveContentsState::HasData,
            ),
        ];
        let list = DriveList::from_assessments(&assessments);
        assert_eq!(list.allowed.len(), 2);
        assert_eq!(list.blocked.len(), 2);

        let sandisk = &list.allowed[0];
        assert_eq!(sandisk.model, "SanDisk Ultra 64GB");
        assert_eq!(sandisk.size, "59.6 GB");
        assert_eq!(sandisk.bus, "USB");
        assert_eq!(sandisk.media, "Removable media");
        assert_eq!(sandisk.status, Status::Ready);
        assert_eq!(sandisk.kind, DriveKind::Usb);

        let card = &list.allowed[1];
        assert_eq!(card.kind, DriveKind::SdCard);
        assert_eq!(card.media, "SD card");
        assert_eq!(card.status, Status::NeedsConfirmation);
        assert_eq!(card.status_label, "Needs confirmation");
        assert_eq!(card.status_detail, "Has files on it");

        assert_eq!(list.blocked[0].size, "932 GB");
        assert_eq!(
            list.blocked[1].reason,
            "External SSD/HDD — usually holds data"
        );
        assert_eq!(list.blocked[1].kind, DriveKind::Disk);
    }

    #[test]
    fn unreadable_contents_are_named() {
        let list = DriveList::from_assessments(&[with_contents(
            assessment(
                3,
                "Kingston",
                1 << 34,
                TargetAssessmentStatus::NeedsConfirmation,
                "",
            ),
            DriveContentsState::Unknown,
        )]);
        assert_eq!(list.allowed[0].status_detail, "Couldn't check what's on it");
    }

    #[test]
    fn a_chosen_drive_is_found_again_only_with_the_same_identity() {
        let mut stick = assessment(2, "SanDisk", 1 << 36, TargetAssessmentStatus::Eligible, "");
        let list = DriveList::from_assessments(std::slice::from_ref(&stick));
        let chosen = list.allowed[0].chosen();
        assert_eq!(list.position_of(&chosen), Some(0));

        stick.identity_code = Some("0000".to_string());
        let list = DriveList::from_assessments(&[stick]);
        assert_eq!(list.position_of(&chosen), None);
    }

    #[test]
    fn reasons_buses_and_media_in_plain_words() {
        assert_eq!(
            blocked_reason("target does not report removable media"),
            "Reports itself as a fixed disk"
        );
        assert_eq!(
            blocked_reason("target size is unknown"),
            "Size could not be read"
        );
        assert_eq!(blocked_reason("target is odd"), "Is odd");
        assert_eq!(bus_label(Some("usb")), "USB");
        assert_eq!(bus_label(Some("NVMe")), "NVMe");
        assert_eq!(bus_label(None), "–");
        assert_eq!(
            media_label(Some("External hard disk media"), DriveKind::Disk),
            "External disk"
        );
        assert_eq!(
            media_label(Some("Fixed hard disk media"), DriveKind::Disk),
            "Fixed disk"
        );
        assert_eq!(media_label(None, DriveKind::Usb), "Unknown");
    }

    #[test]
    fn hints_follow_the_frames() {
        let empty = DriveList::default();
        assert_eq!(
            drive_hint(Some(&empty), None),
            "Waiting for a USB stick or SD card."
        );
        let chosen = ChosenDrive {
            index: 2,
            identity: None,
            model: "SanDisk Ultra 64GB".into(),
        };
        assert_eq!(
            drive_hint(Some(&empty), Some(&chosen)),
            "Selected: SanDisk Ultra 64GB"
        );
        assert_eq!(drive_hint(None, None), "Looking for drives…");
    }
}
