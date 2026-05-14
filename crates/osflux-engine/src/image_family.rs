use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::image_plan::{
    plan_image, DetectionMethod, ImageKind, ImagePlan, ImagePlanError, ImagePlanStatus,
};

pub const DEFAULT_FAMILY_SCAN_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFamilyReport {
    pub status: ImageFamilyStatus,
    pub family: ImageFamily,
    pub reason: String,
    pub image_source: String,
    pub image_kind: ImageKind,
    pub image_status: ImagePlanStatus,
    pub detection_method: DetectionMethod,
    pub scanned_bytes: u64,
    pub scan_limit_bytes: u64,
    pub evidence: Vec<FamilyEvidence>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl ImageFamilyReport {
    pub fn to_json_pretty(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"status\": \"{}\",\n",
                "  \"family\": \"{}\",\n",
                "  \"reason\": \"{}\",\n",
                "  \"imageSource\": \"{}\",\n",
                "  \"imageKind\": \"{}\",\n",
                "  \"imageStatus\": \"{}\",\n",
                "  \"detectionMethod\": \"{}\",\n",
                "  \"scannedBytes\": {},\n",
                "  \"scanLimitBytes\": {},\n",
                "  \"evidence\": {},\n",
                "  \"notes\": {},\n",
                "  \"warnings\": {}\n",
                "}}\n"
            ),
            self.status.as_str(),
            self.family.as_str(),
            escape_json_string(&self.reason),
            escape_json_string(&self.image_source),
            self.image_kind.as_str(),
            self.image_status.as_str(),
            self.detection_method.as_str(),
            self.scanned_bytes,
            self.scan_limit_bytes,
            evidence_json(&self.evidence),
            string_array_json(&self.notes),
            string_array_json(&self.warnings),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFamilyStatus {
    Detected,
    NeedsInspection,
    Blocked,
}

impl ImageFamilyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::NeedsInspection => "needs-inspection",
            Self::Blocked => "blocked",
        }
    }
}

impl fmt::Display for ImageFamilyStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFamily {
    WindowsInstaller,
    LinuxInstaller,
    MultiFamilyImage,
    RawDiskImage,
    Unknown,
}

impl ImageFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowsInstaller => "windows-installer",
            Self::LinuxInstaller => "linux-installer",
            Self::MultiFamilyImage => "multi-family-image",
            Self::RawDiskImage => "raw-disk-image",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for ImageFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyEvidence {
    pub family: ImageFamily,
    pub marker: String,
    pub description: String,
}

#[derive(Debug)]
pub enum ImageFamilyError {
    Image(ImagePlanError),
    Scan { path: PathBuf, message: String },
}

impl fmt::Display for ImageFamilyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => write!(formatter, "image planning failed: {error}"),
            Self::Scan { path, message } => {
                write!(
                    formatter,
                    "could not scan image family markers for {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ImageFamilyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Image(error) => Some(error),
            Self::Scan { .. } => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FamilyMarkerScan {
    pub scanned_bytes: u64,
    pub scan_limit_bytes: u64,
    pub evidence: Vec<FamilyEvidence>,
}

pub fn detect_image_family(path: impl AsRef<Path>) -> Result<ImageFamilyReport, ImageFamilyError> {
    let image_plan = plan_image(path.as_ref()).map_err(ImageFamilyError::Image)?;
    let scan = if image_plan.kind == ImageKind::Iso && image_plan.size_bytes > 0 {
        scan_family_markers(&image_plan.source, DEFAULT_FAMILY_SCAN_LIMIT_BYTES).map_err(
            |error| ImageFamilyError::Scan {
                path: image_plan.source.clone(),
                message: error.to_string(),
            },
        )?
    } else {
        FamilyMarkerScan {
            scanned_bytes: 0,
            scan_limit_bytes: DEFAULT_FAMILY_SCAN_LIMIT_BYTES,
            evidence: Vec::new(),
        }
    };

    Ok(build_image_family_report(&image_plan, scan))
}

pub fn build_image_family_report(
    image_plan: &ImagePlan,
    scan: FamilyMarkerScan,
) -> ImageFamilyReport {
    let family = classify_family(image_plan, &scan.evidence);
    let status = classify_family_status(image_plan, family);
    let mut notes = Vec::new();
    let mut warnings = image_plan.warnings.clone();

    notes.push("Image family detection is read-only; no target access was attempted.".to_string());
    notes.push(format!(
        "Image kind from planner: {}.",
        image_plan.kind.as_str()
    ));

    if image_plan.kind == ImageKind::Iso {
        notes.push(format!(
            "Scanned {} byte(s) for image family markers.",
            scan.scanned_bytes
        ));
    }

    if image_plan.status == ImagePlanStatus::NeedsInspection {
        warnings.push(
            "Image planner still requires deeper inspection before write execution.".to_string(),
        );
    }

    if status == ImageFamilyStatus::NeedsInspection {
        warnings.push("Image family could not be confidently detected yet.".to_string());
    }

    ImageFamilyReport {
        status,
        family,
        reason: family_reason(image_plan, family, status).to_string(),
        image_source: image_plan.source.display().to_string(),
        image_kind: image_plan.kind,
        image_status: image_plan.status,
        detection_method: image_plan.detection_method,
        scanned_bytes: scan.scanned_bytes,
        scan_limit_bytes: scan.scan_limit_bytes,
        evidence: scan.evidence,
        notes,
        warnings,
    }
}

fn classify_family(image_plan: &ImagePlan, evidence: &[FamilyEvidence]) -> ImageFamily {
    if image_plan.kind == ImageKind::RawDiskImage {
        return ImageFamily::RawDiskImage;
    }

    let has_windows = evidence
        .iter()
        .any(|item| item.family == ImageFamily::WindowsInstaller);
    let has_linux = evidence
        .iter()
        .any(|item| item.family == ImageFamily::LinuxInstaller);

    match (has_windows, has_linux) {
        (true, true) => ImageFamily::MultiFamilyImage,
        (true, false) => ImageFamily::WindowsInstaller,
        (false, true) => ImageFamily::LinuxInstaller,
        (false, false) => ImageFamily::Unknown,
    }
}

fn classify_family_status(image_plan: &ImagePlan, family: ImageFamily) -> ImageFamilyStatus {
    if image_plan.status == ImagePlanStatus::Blocked {
        return ImageFamilyStatus::Blocked;
    }

    if family == ImageFamily::Unknown {
        return ImageFamilyStatus::NeedsInspection;
    }

    ImageFamilyStatus::Detected
}

fn family_reason(
    image_plan: &ImagePlan,
    family: ImageFamily,
    status: ImageFamilyStatus,
) -> &'static str {
    if image_plan.status == ImagePlanStatus::Blocked {
        return "image family detection blocked by image plan";
    }

    match (status, family) {
        (ImageFamilyStatus::Detected, ImageFamily::WindowsInstaller) => {
            "detected Windows installer family markers"
        }
        (ImageFamilyStatus::Detected, ImageFamily::LinuxInstaller) => {
            "detected Linux installer family markers"
        }
        (ImageFamilyStatus::Detected, ImageFamily::MultiFamilyImage) => {
            "detected markers from multiple image families"
        }
        (ImageFamilyStatus::Detected, ImageFamily::RawDiskImage) => {
            "raw disk image family inferred from image kind"
        }
        (ImageFamilyStatus::NeedsInspection, ImageFamily::Unknown) => {
            "image family needs deeper inspection"
        }
        _ => "image family detection did not reach a confident result",
    }
}

fn scan_family_markers(path: &Path, scan_limit_bytes: u64) -> std::io::Result<FamilyMarkerScan> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    let mut limited = (&mut file).take(scan_limit_bytes);
    limited.read_to_end(&mut buffer)?;
    let scanned_bytes = buffer.len() as u64;
    buffer.make_ascii_lowercase();

    let evidence = marker_definitions()
        .iter()
        .filter(|marker| contains_bytes(&buffer, marker.pattern))
        .map(|marker| FamilyEvidence {
            family: marker.family,
            marker: marker.label.to_string(),
            description: marker.description.to_string(),
        })
        .collect();

    Ok(FamilyMarkerScan {
        scanned_bytes,
        scan_limit_bytes,
        evidence,
    })
}

struct MarkerDefinition {
    family: ImageFamily,
    pattern: &'static [u8],
    label: &'static str,
    description: &'static str,
}

fn marker_definitions() -> &'static [MarkerDefinition] {
    &[
        MarkerDefinition {
            family: ImageFamily::WindowsInstaller,
            pattern: b"bootmgr",
            label: "bootmgr",
            description: "Windows boot manager marker",
        },
        MarkerDefinition {
            family: ImageFamily::WindowsInstaller,
            pattern: b"install.wim",
            label: "install.wim",
            description: "Windows install image marker",
        },
        MarkerDefinition {
            family: ImageFamily::WindowsInstaller,
            pattern: b"install.esd",
            label: "install.esd",
            description: "Windows compressed install image marker",
        },
        MarkerDefinition {
            family: ImageFamily::WindowsInstaller,
            pattern: b"efi/microsoft/boot",
            label: "efi/microsoft/boot",
            description: "Windows UEFI boot path marker",
        },
        MarkerDefinition {
            family: ImageFamily::LinuxInstaller,
            pattern: b"isolinux",
            label: "isolinux",
            description: "Linux ISO bootloader marker",
        },
        MarkerDefinition {
            family: ImageFamily::LinuxInstaller,
            pattern: b"vmlinuz",
            label: "vmlinuz",
            description: "Linux kernel image marker",
        },
        MarkerDefinition {
            family: ImageFamily::LinuxInstaller,
            pattern: b"initrd",
            label: "initrd",
            description: "Linux initrd marker",
        },
        MarkerDefinition {
            family: ImageFamily::LinuxInstaller,
            pattern: b"squashfs",
            label: "squashfs",
            description: "Linux live filesystem marker",
        },
        MarkerDefinition {
            family: ImageFamily::LinuxInstaller,
            pattern: b"boot/grub",
            label: "boot/grub",
            description: "GRUB boot path marker",
        },
    ]
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }

    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn evidence_json(values: &[FamilyEvidence]) -> String {
    let values = values
        .iter()
        .map(|value| {
            format!(
                concat!(
                    "{{",
                    "\"family\":\"{}\",",
                    "\"marker\":\"{}\",",
                    "\"description\":\"{}\"",
                    "}}"
                ),
                value.family.as_str(),
                escape_json_string(&value.marker),
                escape_json_string(&value.description)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{values}]")
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
    use super::{detect_image_family, ImageFamily, ImageFamilyStatus};
    use std::fs;

    #[test]
    fn detects_windows_installer_markers() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"OSFLUX TEST IMAGE bootmgr sources/install.wim").unwrap();

        let report = detect_image_family(&path).unwrap();

        assert_eq!(report.status, ImageFamilyStatus::Detected);
        assert_eq!(report.family, ImageFamily::WindowsInstaller);
        assert!(report.evidence.iter().any(|item| item.marker == "bootmgr"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn detects_linux_installer_markers() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"OSFLUX TEST IMAGE isolinux vmlinuz initrd squashfs").unwrap();

        let report = detect_image_family(&path).unwrap();

        assert_eq!(report.status, ImageFamilyStatus::Detected);
        assert_eq!(report.family, ImageFamily::LinuxInstaller);
        assert!(report.evidence.iter().any(|item| item.marker == "vmlinuz"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn detects_multiple_family_markers() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"bootmgr install.wim isolinux vmlinuz").unwrap();

        let report = detect_image_family(&path).unwrap();

        assert_eq!(report.status, ImageFamilyStatus::Detected);
        assert_eq!(report.family, ImageFamily::MultiFamilyImage);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn infers_raw_disk_family_from_image_kind() {
        let path = std::env::temp_dir().join(unique_name("img"));
        fs::write(&path, b"raw disk bytes").unwrap();

        let report = detect_image_family(&path).unwrap();

        assert_eq!(report.status, ImageFamilyStatus::Detected);
        assert_eq!(report.family, ImageFamily::RawDiskImage);
        assert_eq!(report.scanned_bytes, 0);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn needs_inspection_for_unknown_iso_family() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"no known family marker").unwrap();

        let report = detect_image_family(&path).unwrap();

        assert_eq!(report.status, ImageFamilyStatus::NeedsInspection);
        assert_eq!(report.family, ImageFamily::Unknown);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn serializes_family_report_as_json() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"bootmgr install.wim").unwrap();

        let json = detect_image_family(&path).unwrap().to_json_pretty();

        assert!(json.contains("\"status\": \"detected\""));
        assert!(json.contains("\"family\": \"windows-installer\""));
        assert!(json.contains("\"marker\":\"bootmgr\""));

        let _ = fs::remove_file(path);
    }

    fn unique_name(extension: &str) -> String {
        format!(
            "osflux-family-test-{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        )
    }
}
