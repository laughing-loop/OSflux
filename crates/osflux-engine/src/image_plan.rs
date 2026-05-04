use std::fmt;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::iso::{inspect_boot_metadata, IsoBootMetadata};

const ISO_SECTOR_SIZE: u64 = 2048;
const ISO_VOLUME_DESCRIPTOR_START: u64 = 16 * ISO_SECTOR_SIZE;
const GPT_HEADER_OFFSET: u64 = 512;
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const MBR_SIGNATURE_OFFSET: u64 = 510;
const MBR_SIGNATURE: &[u8; 2] = &[0x55, 0xAA];
const VOLUME_DESCRIPTOR_IDENTIFIER_OFFSET: u64 = 1;
const VOLUME_DESCRIPTOR_IDENTIFIER_SIZE: usize = 5;
const VOLUME_DESCRIPTOR_SCAN_SECTORS: u64 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlan {
    pub source: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub kind: ImageKind,
    pub detection_method: DetectionMethod,
    pub signatures: Vec<ImageSignature>,
    pub boot_metadata: Option<IsoBootMetadata>,
    pub write_mode: WriteMode,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Iso,
    RawDiskImage,
    Unknown,
}

impl ImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Iso => "iso",
            Self::RawDiskImage => "raw-disk-image",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionMethod {
    Signature,
    Extension,
    Unknown,
}

impl DetectionMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signature => "signature",
            Self::Extension => "extension",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSignature {
    Iso9660,
    Udf,
    Gpt,
    Mbr,
}

impl ImageSignature {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Iso9660 => "ISO 9660 volume descriptor",
            Self::Udf => "UDF volume descriptor",
            Self::Gpt => "GPT header",
            Self::Mbr => "MBR boot signature",
        }
    }

    fn implies_kind(self) -> ImageKind {
        match self {
            Self::Iso9660 | Self::Udf => ImageKind::Iso,
            Self::Gpt | Self::Mbr => ImageKind::RawDiskImage,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    FileSystem,
    Raw,
    Unknown,
}

impl WriteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileSystem => "filesystem",
            Self::Raw => "raw",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImagePlanError {
    NotFound(PathBuf),
    NotFile(PathBuf),
    Metadata { path: PathBuf, message: String },
    Inspect { path: PathBuf, message: String },
}

impl fmt::Display for ImagePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(path) => {
                write!(formatter, "image path does not exist: {}", path.display())
            }
            Self::NotFile(path) => {
                write!(formatter, "image path is not a file: {}", path.display())
            }
            Self::Metadata { path, message } => {
                write!(
                    formatter,
                    "could not read metadata for {}: {message}",
                    path.display()
                )
            }
            Self::Inspect { path, message } => {
                write!(
                    formatter,
                    "could not inspect image signatures for {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ImagePlanError {}

pub fn plan_image(path: impl AsRef<Path>) -> Result<ImagePlan, ImagePlanError> {
    let source = path.as_ref().to_path_buf();

    if !source.exists() {
        return Err(ImagePlanError::NotFound(source));
    }

    let metadata = source
        .metadata()
        .map_err(|error| ImagePlanError::Metadata {
            path: source.clone(),
            message: error.to_string(),
        })?;

    if !metadata.is_file() {
        return Err(ImagePlanError::NotFile(source));
    }

    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("<unnamed>")
        .to_string();

    let signatures = inspect_signatures(&source).map_err(|error| ImagePlanError::Inspect {
        path: source.clone(),
        message: error.to_string(),
    })?;
    let extension_kind = classify_by_extension(&source);
    let (kind, detection_method) = classify_image(extension_kind, &signatures);
    let write_mode = write_mode_for(kind);
    let mut boot_metadata = None;
    let mut notes = Vec::new();
    let mut warnings = Vec::new();

    if !signatures.is_empty() {
        notes.push(
            "Image was classified from file signatures before extension fallback.".to_string(),
        );
    } else if detection_method == DetectionMethod::Extension {
        notes.push("No known image signature was found; extension fallback was used.".to_string());
    }

    match kind {
        ImageKind::Iso => {
            match inspect_boot_metadata(&source) {
                Ok(metadata) => {
                    if metadata.el_torito.is_some() {
                        notes.push("El Torito boot catalog detected.".to_string());
                    } else {
                        warnings
                            .push("No El Torito boot catalog was found in this ISO.".to_string());
                    }

                    boot_metadata = Some(metadata);
                }
                Err(error) => {
                    warnings.push(format!("Could not inspect ISO boot metadata: {error}"));
                }
            }
            notes.push("Initial mode is filesystem planning because ISO content often needs extraction and boot setup.".to_string());
        }
        ImageKind::RawDiskImage => {
            notes.push("Raw disk image detected; initial mode is direct block writing after target safety checks.".to_string());
        }
        ImageKind::Unknown => {
            warnings.push("Image type is unknown; OSflux will require deeper inspection before write support.".to_string());
        }
    }

    if detection_method == DetectionMethod::Extension {
        warnings
            .push("Extension-based classification is weaker than signature detection.".to_string());
    }

    if metadata.len() == 0 {
        warnings.push("Image file is empty.".to_string());
    }

    Ok(ImagePlan {
        source,
        file_name,
        size_bytes: metadata.len(),
        kind,
        detection_method,
        signatures,
        boot_metadata,
        write_mode,
        notes,
        warnings,
    })
}

fn inspect_signatures(path: &Path) -> std::io::Result<Vec<ImageSignature>> {
    let mut file = File::open(path)?;
    let mut signatures = Vec::new();

    if has_iso9660_signature(&mut file)? {
        signatures.push(ImageSignature::Iso9660);
    }

    if has_udf_signature(&mut file)? {
        signatures.push(ImageSignature::Udf);
    }

    if has_gpt_signature(&mut file)? {
        signatures.push(ImageSignature::Gpt);
    }

    if has_mbr_signature(&mut file)? {
        signatures.push(ImageSignature::Mbr);
    }

    Ok(signatures)
}

fn has_iso9660_signature(file: &mut File) -> std::io::Result<bool> {
    read_exact_at_optional(
        file,
        ISO_VOLUME_DESCRIPTOR_START + VOLUME_DESCRIPTOR_IDENTIFIER_OFFSET,
    )
    .map(|buffer: Option<[u8; VOLUME_DESCRIPTOR_IDENTIFIER_SIZE]>| {
        buffer.is_some_and(|buffer| &buffer == b"CD001")
    })
}

fn has_udf_signature(file: &mut File) -> std::io::Result<bool> {
    for sector in 16..VOLUME_DESCRIPTOR_SCAN_SECTORS {
        let offset = (sector * ISO_SECTOR_SIZE) + VOLUME_DESCRIPTOR_IDENTIFIER_OFFSET;
        let Some(identifier) = read_exact_at_optional(file, offset)? else {
            continue;
        };

        if matches!(&identifier, b"BEA01" | b"NSR02" | b"NSR03" | b"TEA01") {
            return Ok(true);
        }
    }

    Ok(false)
}

fn has_gpt_signature(file: &mut File) -> std::io::Result<bool> {
    read_exact_at_optional(file, GPT_HEADER_OFFSET)
        .map(|buffer: Option<[u8; 8]>| buffer.is_some_and(|buffer| &buffer == GPT_SIGNATURE))
}

fn has_mbr_signature(file: &mut File) -> std::io::Result<bool> {
    read_exact_at_optional(file, MBR_SIGNATURE_OFFSET)
        .map(|buffer: Option<[u8; 2]>| buffer.is_some_and(|buffer| &buffer == MBR_SIGNATURE))
}

fn read_exact_at_optional<const N: usize>(
    file: &mut File,
    offset: u64,
) -> std::io::Result<Option<[u8; N]>> {
    let mut buffer = [0u8; N];
    file.seek(SeekFrom::Start(offset))?;

    match file.read_exact(&mut buffer) {
        Ok(()) => Ok(Some(buffer)),
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

fn classify_image(
    extension_kind: ImageKind,
    signatures: &[ImageSignature],
) -> (ImageKind, DetectionMethod) {
    if let Some(kind) = classify_by_signature(signatures) {
        return (kind, DetectionMethod::Signature);
    }

    if extension_kind != ImageKind::Unknown {
        return (extension_kind, DetectionMethod::Extension);
    }

    (ImageKind::Unknown, DetectionMethod::Unknown)
}

fn classify_by_signature(signatures: &[ImageSignature]) -> Option<ImageKind> {
    signatures
        .iter()
        .copied()
        .find(|signature| matches!(signature, ImageSignature::Iso9660 | ImageSignature::Udf))
        .or_else(|| signatures.first().copied())
        .map(ImageSignature::implies_kind)
}

fn classify_by_extension(path: &Path) -> ImageKind {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "iso" => ImageKind::Iso,
        "img" | "raw" | "bin" => ImageKind::RawDiskImage,
        _ => ImageKind::Unknown,
    }
}

fn write_mode_for(kind: ImageKind) -> WriteMode {
    match kind {
        ImageKind::Iso => WriteMode::FileSystem,
        ImageKind::RawDiskImage => WriteMode::Raw,
        ImageKind::Unknown => WriteMode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_image, DetectionMethod, ImageKind, ImageSignature, WriteMode};
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn classifies_iso_images_by_extension_fallback() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"not a real iso yet").unwrap();

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Iso);
        assert_eq!(plan.detection_method, DetectionMethod::Extension);
        assert_eq!(plan.write_mode, WriteMode::FileSystem);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn classifies_raw_images_by_extension_fallback() {
        let path = std::env::temp_dir().join(unique_name("img"));
        fs::write(&path, b"raw data").unwrap();

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::RawDiskImage);
        assert_eq!(plan.detection_method, DetectionMethod::Extension);
        assert_eq!(plan.write_mode, WriteMode::Raw);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn classifies_iso_images_by_signature() {
        let path = std::env::temp_dir().join(unique_name("bin"));
        write_at(&path, 0x8001, b"CD001");

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Iso);
        assert_eq!(plan.detection_method, DetectionMethod::Signature);
        assert_eq!(plan.signatures, vec![ImageSignature::Iso9660]);
        assert_eq!(plan.write_mode, WriteMode::FileSystem);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn classifies_gpt_images_by_signature() {
        let path = std::env::temp_dir().join(unique_name("unknown"));
        write_at(&path, 512, b"EFI PART");

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::RawDiskImage);
        assert_eq!(plan.detection_method, DetectionMethod::Signature);
        assert_eq!(plan.signatures, vec![ImageSignature::Gpt]);
        assert_eq!(plan.write_mode, WriteMode::Raw);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prefers_iso_signature_over_raw_signature() {
        let path = std::env::temp_dir().join(unique_name("img"));
        write_at(&path, 510, &[0x55, 0xAA]);
        write_at(&path, 0x8001, b"CD001");

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Iso);
        assert_eq!(plan.detection_method, DetectionMethod::Signature);
        assert_eq!(
            plan.signatures,
            vec![ImageSignature::Iso9660, ImageSignature::Mbr]
        );
        assert_eq!(plan.write_mode, WriteMode::FileSystem);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_missing_images() {
        let path = std::env::temp_dir().join(unique_name("iso"));

        let error = plan_image(&path).unwrap_err();

        assert!(error.to_string().contains("does not exist"));
    }

    fn write_at(path: &std::path::Path, offset: u64, bytes: &[u8]) {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .unwrap();

        file.seek(SeekFrom::Start(offset)).unwrap();
        file.write_all(bytes).unwrap();
    }

    fn unique_name(extension: &str) -> String {
        format!(
            "osflux-test-{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        )
    }
}
