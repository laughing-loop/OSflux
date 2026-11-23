use std::fmt;
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::file_copy::{summarize_iso_files, IsoFilesSummary};

use crate::iso::{inspect_boot_metadata, BootStyle, IsoBootMetadata};

const ISO_SECTOR_SIZE: u64 = 2048;
const NO_ISO9660_BOOT_ASSETS: &str =
    "No known boot asset paths were found in ISO directory records.";
const ISO_VOLUME_DESCRIPTOR_START: u64 = 16 * ISO_SECTOR_SIZE;
const GPT_HEADER_OFFSET: u64 = 512;
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const MBR_SIGNATURE_OFFSET: u64 = 510;
const MBR_SIGNATURE: &[u8; 2] = &[0x55, 0xAA];
const MBR_PARTITION_TABLE_OFFSET: usize = 446;
const MBR_PARTITION_ENTRY_SIZE: usize = 16;
const MBR_PARTITION_ENTRIES: usize = 4;
const VOLUME_DESCRIPTOR_IDENTIFIER_OFFSET: u64 = 1;
const VOLUME_DESCRIPTOR_IDENTIFIER_SIZE: usize = 5;
const VOLUME_DESCRIPTOR_SCAN_SECTORS: u64 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlan {
    pub source: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub status: ImagePlanStatus,
    pub status_reason: String,
    pub kind: ImageKind,
    pub detection_method: DetectionMethod,
    pub signatures: Vec<ImageSignature>,
    /// An ISO whose first sector also holds a partition table, so it can be
    /// copied to a drive byte for byte and boot from there.
    pub hybrid: bool,
    pub boot_metadata: Option<IsoBootMetadata>,
    pub write_mode: WriteMode,
    pub write_strategy: ImageWriteStrategy,
    /// For an ISO whose files would be copied: what its UDF filesystem
    /// holds, or why it could not be read. `None` for other images.
    pub iso_files: Option<Result<IsoFilesSummary, String>>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePlanStatus {
    Ready,
    NeedsInspection,
    Blocked,
}

impl ImagePlanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsInspection => "needs-inspection",
            Self::Blocked => "blocked",
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageWriteStrategy {
    IsoFileSystemBootSetup,
    IsoNeedsDeeperInspection,
    RawBlockWrite,
    HybridIsoRawWrite,
    Unknown,
}

impl ImageWriteStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IsoFileSystemBootSetup => "iso-filesystem-boot-setup",
            Self::IsoNeedsDeeperInspection => "iso-needs-deeper-inspection",
            Self::RawBlockWrite => "raw-block-write",
            Self::HybridIsoRawWrite => "hybrid-iso-raw-write",
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
    let hybrid = kind == ImageKind::Iso
        && has_hybrid_partition_table(&source, &signatures).map_err(|error| {
            ImagePlanError::Inspect {
                path: source.clone(),
                message: error.to_string(),
            }
        })?;
    let write_mode = write_mode_for(kind, hybrid);
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
                    if let Some(info) = metadata.el_torito.as_ref() {
                        notes.push("El Torito boot catalog detected.".to_string());
                        if !info.validation_entry_valid {
                            warnings.push("El Torito validation entry is invalid.".to_string());
                        }

                        if metadata.boot_image_ranges_valid() {
                            notes.push("Boot image byte ranges fit inside the ISO.".to_string());
                        } else {
                            warnings.push(
                                "One or more ISO boot entries could not be fully range-validated."
                                    .to_string(),
                            );
                        }
                    } else {
                        warnings
                            .push("No El Torito boot catalog was found in this ISO.".to_string());
                    }

                    if let Some(directory) = metadata.directory.as_ref() {
                        notes.push(format!(
                            "ISO directory records scanned: {} directories, {} entries.",
                            directory.scanned_directories,
                            directory.entries.len()
                        ));

                        if directory.boot_assets.is_empty() {
                            warnings.push(NO_ISO9660_BOOT_ASSETS.to_string());
                        } else {
                            notes.push(format!(
                                "ISO directory records exposed {} boot asset path(s).",
                                directory.boot_assets.len()
                            ));
                        }

                        if directory.truncated {
                            warnings.push(
                                "ISO directory inspection reached a safety limit and was truncated."
                                    .to_string(),
                            );
                        }
                    } else {
                        warnings.push(
                            "Could not read the ISO primary directory record yet.".to_string(),
                        );
                    }

                    boot_metadata = Some(metadata);
                }
                Err(error) => {
                    warnings.push(format!("Could not inspect ISO boot metadata: {error}"));
                }
            }
            if hybrid {
                notes.push("ISO is hybrid: its first sector also holds a partition table, so it can be written to a drive byte for byte.".to_string());
            } else {
                notes.push("Initial mode is filesystem planning because ISO content often needs extraction and boot setup.".to_string());
            }
        }
        ImageKind::RawDiskImage => {
            notes.push("Raw disk image detected; initial mode is direct block writing after target safety checks.".to_string());
        }
        ImageKind::Unknown => {
            warnings.push("Image type is unknown; OSflux will require deeper inspection before write support.".to_string());
        }
    }

    let write_strategy = write_strategy_for(kind, hybrid, boot_metadata.as_ref());
    add_strategy_notes(write_strategy, &mut notes, &mut warnings);

    if detection_method == DetectionMethod::Extension {
        warnings
            .push("Extension-based classification is weaker than signature detection.".to_string());
    }

    if metadata.len() == 0 {
        warnings.push("Image file is empty.".to_string());
    }

    let status = status_for(kind, detection_method, write_strategy, metadata.len());
    let status_reason = status_reason_for(
        status,
        kind,
        detection_method,
        write_strategy,
        metadata.len(),
    )
    .to_string();

    // Files are only copied out of ISOs that are not hybrid.
    let iso_files = (kind == ImageKind::Iso && !hybrid)
        .then(|| summarize_iso_files(&source))
        .flatten();
    if let Some(Ok(summary)) = &iso_files {
        for reason in &summary.cannot_copy {
            warnings.push(format!("This ISO cannot be written: {reason}."));
        }
        match &summary.uefi_loader {
            Some(loader) => {
                // Windows ISOs list only a README in their ISO 9660 directory,
                // so that directory having no boot files is expected.
                if warnings.iter().any(|warning| warning == NO_ISO9660_BOOT_ASSETS) {
                    warnings.retain(|warning| warning != NO_ISO9660_BOOT_ASSETS);
                    notes.push(format!(
                        "The boot files are in the ISO's UDF filesystem, not its ISO 9660 directory; UEFI loader: {loader}."
                    ));
                }
            }
            None => warnings.push(
                "The ISO has no UEFI boot loader (efi/boot/boot*.efi), so a stick made from it could not start a PC."
                    .to_string(),
            ),
        }
    }

    Ok(ImagePlan {
        source,
        file_name,
        size_bytes: metadata.len(),
        status,
        status_reason,
        kind,
        detection_method,
        signatures,
        hybrid,
        boot_metadata,
        write_mode,
        write_strategy,
        iso_files,
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

/// A hybrid ISO carries a partition table in its first sector as well as its
/// ISO 9660 filesystem, which is what lets it boot when copied to a USB drive
/// byte for byte. The MBR boot signature alone is not enough: there must also
/// be a GPT header or at least one MBR partition entry with a size.
fn has_hybrid_partition_table(path: &Path, signatures: &[ImageSignature]) -> std::io::Result<bool> {
    if !signatures.contains(&ImageSignature::Mbr) {
        return Ok(false);
    }

    if signatures.contains(&ImageSignature::Gpt) {
        return Ok(true);
    }

    let mut file = File::open(path)?;
    let Some(sector) = read_exact_at_optional::<512>(&mut file, 0)? else {
        return Ok(false);
    };
    let table_end = MBR_PARTITION_TABLE_OFFSET + MBR_PARTITION_ENTRIES * MBR_PARTITION_ENTRY_SIZE;

    Ok(sector[MBR_PARTITION_TABLE_OFFSET..table_end]
        .chunks_exact(MBR_PARTITION_ENTRY_SIZE)
        .any(|entry| u32::from_le_bytes(entry[12..16].try_into().unwrap()) != 0))
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

fn write_mode_for(kind: ImageKind, hybrid: bool) -> WriteMode {
    match kind {
        ImageKind::Iso if hybrid => WriteMode::Raw,
        ImageKind::Iso => WriteMode::FileSystem,
        ImageKind::RawDiskImage => WriteMode::Raw,
        ImageKind::Unknown => WriteMode::Unknown,
    }
}

fn write_strategy_for(
    kind: ImageKind,
    hybrid: bool,
    boot_metadata: Option<&IsoBootMetadata>,
) -> ImageWriteStrategy {
    match kind {
        // A hybrid ISO boots from its own partition table once copied, so it
        // does not depend on the El Torito metadata that extraction would use.
        ImageKind::Iso if hybrid => ImageWriteStrategy::HybridIsoRawWrite,
        ImageKind::Iso => match boot_metadata {
            Some(metadata) if iso_metadata_ready_for_boot_setup(metadata) => {
                ImageWriteStrategy::IsoFileSystemBootSetup
            }
            _ => ImageWriteStrategy::IsoNeedsDeeperInspection,
        },
        ImageKind::RawDiskImage => ImageWriteStrategy::RawBlockWrite,
        ImageKind::Unknown => ImageWriteStrategy::Unknown,
    }
}

fn iso_metadata_ready_for_boot_setup(metadata: &IsoBootMetadata) -> bool {
    let Some(info) = metadata.el_torito.as_ref() else {
        return false;
    };

    info.validation_entry_valid
        && matches!(
            metadata.boot_style(),
            BootStyle::Bios | BootStyle::Uefi | BootStyle::Hybrid
        )
        && metadata.boot_image_ranges_valid()
}

fn add_strategy_notes(
    write_strategy: ImageWriteStrategy,
    notes: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    match write_strategy {
        ImageWriteStrategy::IsoFileSystemBootSetup => {
            notes.push("Write strategy selected: extract ISO content and prepare boot files from validated metadata.".to_string());
        }
        ImageWriteStrategy::IsoNeedsDeeperInspection => {
            warnings.push(
                "Write strategy is blocked until ISO boot metadata is complete and valid."
                    .to_string(),
            );
        }
        ImageWriteStrategy::RawBlockWrite => {
            notes.push(
                "Write strategy selected: direct block write after device safety confirmation."
                    .to_string(),
            );
        }
        ImageWriteStrategy::HybridIsoRawWrite => {
            notes.push(
                "Write strategy selected: copy the hybrid ISO to the drive byte for byte after device safety confirmation."
                    .to_string(),
            );
        }
        ImageWriteStrategy::Unknown => {
            warnings.push("No write strategy can be selected for this image yet.".to_string());
        }
    }
}

fn status_for(
    kind: ImageKind,
    detection_method: DetectionMethod,
    write_strategy: ImageWriteStrategy,
    size_bytes: u64,
) -> ImagePlanStatus {
    if size_bytes == 0
        || kind == ImageKind::Unknown
        || write_strategy == ImageWriteStrategy::Unknown
    {
        return ImagePlanStatus::Blocked;
    }

    if detection_method == DetectionMethod::Extension
        || write_strategy == ImageWriteStrategy::IsoNeedsDeeperInspection
    {
        return ImagePlanStatus::NeedsInspection;
    }

    ImagePlanStatus::Ready
}

fn status_reason_for(
    status: ImagePlanStatus,
    kind: ImageKind,
    detection_method: DetectionMethod,
    write_strategy: ImageWriteStrategy,
    size_bytes: u64,
) -> &'static str {
    if size_bytes == 0 {
        return "image file is empty";
    }

    if kind == ImageKind::Unknown || write_strategy == ImageWriteStrategy::Unknown {
        return "image type is unknown";
    }

    if detection_method == DetectionMethod::Extension {
        return "image classification used extension fallback";
    }

    if write_strategy == ImageWriteStrategy::IsoNeedsDeeperInspection {
        return "ISO boot metadata needs deeper inspection";
    }

    match status {
        ImagePlanStatus::Ready => "image plan is trusted for the next safety phase",
        ImagePlanStatus::NeedsInspection => "image plan needs deeper inspection",
        ImagePlanStatus::Blocked => "image plan is blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        plan_image, DetectionMethod, ImageKind, ImagePlanStatus, ImageSignature,
        ImageWriteStrategy, WriteMode, ISO_SECTOR_SIZE,
    };
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
        assert_eq!(
            plan.write_strategy,
            ImageWriteStrategy::IsoNeedsDeeperInspection
        );
        assert_eq!(plan.status, ImagePlanStatus::NeedsInspection);

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
        assert_eq!(plan.write_strategy, ImageWriteStrategy::RawBlockWrite);
        assert_eq!(plan.status, ImagePlanStatus::NeedsInspection);

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
        assert_eq!(
            plan.write_strategy,
            ImageWriteStrategy::IsoNeedsDeeperInspection
        );
        assert_eq!(plan.status, ImagePlanStatus::NeedsInspection);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn selects_iso_boot_setup_strategy_for_valid_boot_metadata() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        write_bootable_iso(&path, 20, 24);

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Iso);
        assert_eq!(plan.detection_method, DetectionMethod::Signature);
        assert_eq!(
            plan.write_strategy,
            ImageWriteStrategy::IsoFileSystemBootSetup
        );
        assert_eq!(plan.status, ImagePlanStatus::Ready);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn finds_the_boot_files_of_a_windows_iso_in_udf() {
        // Microsoft's ISOs list only a README in their ISO 9660 directory;
        // the boot files are in UDF, which OSflux reads.
        let image = crate::test_support::SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/isos/windows-udf-only.sparse"
        ));
        let size = crate::image_io::ImageRead::image_len(&image).unwrap() as usize;
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, image.read(0, size)).unwrap();

        let plan = plan_image(&path);
        let _ = fs::remove_file(&path);
        let plan = plan.unwrap();

        assert_eq!(plan.status, ImagePlanStatus::Ready);
        assert_eq!(
            plan.write_strategy,
            ImageWriteStrategy::IsoFileSystemBootSetup
        );
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
        assert!(
            plan.notes
                .iter()
                .any(|note| note.contains("UDF") && note.contains("efi/boot/bootx64.efi")),
            "{:?}",
            plan.notes
        );
    }

    #[test]
    fn blocks_iso_boot_setup_strategy_for_invalid_catalog_validation() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        write_bootable_iso_with_invalid_validation(&path, 20, 24);

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Iso);
        assert_eq!(
            plan.write_strategy,
            ImageWriteStrategy::IsoNeedsDeeperInspection
        );
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("validation entry is invalid")));
        assert_eq!(plan.status, ImagePlanStatus::NeedsInspection);

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
        assert_eq!(plan.write_strategy, ImageWriteStrategy::RawBlockWrite);
        assert_eq!(plan.status, ImagePlanStatus::Ready);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn copies_hybrid_iso_with_mbr_partition_byte_for_byte() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        // Arch and Fedora style: the first MBR entry has partition type 0 and
        // the boot flag, but a real start and size.
        let mut entry = [0u8; 16];
        entry[0] = 0x80;
        entry[8..12].copy_from_slice(&64u32.to_le_bytes());
        entry[12..16].copy_from_slice(&1_590_720u32.to_le_bytes());
        write_at(&path, 446, &entry);
        write_at(&path, 510, &[0x55, 0xAA]);
        write_at(&path, 0x8001, b"CD001");

        let plan = plan_image(&path).unwrap();
        let _ = fs::remove_file(path);

        assert_eq!(plan.kind, ImageKind::Iso);
        assert!(plan.hybrid);
        assert_eq!(plan.write_mode, WriteMode::Raw);
        assert_eq!(plan.write_strategy, ImageWriteStrategy::HybridIsoRawWrite);
        assert_eq!(plan.status, ImagePlanStatus::Ready);
    }

    #[test]
    fn copies_hybrid_iso_with_gpt_byte_for_byte() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        // Ubuntu style: a protective MBR followed by a GPT header.
        write_at(&path, 510, &[0x55, 0xAA]);
        write_at(&path, 512, b"EFI PART");
        write_at(&path, 0x8001, b"CD001");

        let plan = plan_image(&path).unwrap();
        let _ = fs::remove_file(path);

        assert!(plan.hybrid);
        assert_eq!(plan.write_strategy, ImageWriteStrategy::HybridIsoRawWrite);
        assert_eq!(plan.status, ImagePlanStatus::Ready);
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
        // A boot signature with an empty partition table is not a hybrid ISO.
        assert!(!plan.hybrid);
        assert_eq!(plan.write_mode, WriteMode::FileSystem);
        assert_eq!(
            plan.write_strategy,
            ImageWriteStrategy::IsoNeedsDeeperInspection
        );
        assert_eq!(plan.status, ImagePlanStatus::NeedsInspection);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn blocks_unknown_images() {
        let path = std::env::temp_dir().join(unique_name("unknown"));
        fs::write(&path, b"not a recognized image").unwrap();

        let plan = plan_image(&path).unwrap();

        assert_eq!(plan.kind, ImageKind::Unknown);
        assert_eq!(plan.status, ImagePlanStatus::Blocked);
        assert_eq!(plan.status_reason, "image type is unknown");

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

    fn write_bootable_iso(path: &std::path::Path, catalog_lba: u32, load_rba: u32) {
        write_bootable_iso_with_validation(path, catalog_lba, load_rba, true);
    }

    fn write_bootable_iso_with_invalid_validation(
        path: &std::path::Path,
        catalog_lba: u32,
        load_rba: u32,
    ) {
        write_bootable_iso_with_validation(path, catalog_lba, load_rba, false);
    }

    fn write_bootable_iso_with_validation(
        path: &std::path::Path,
        catalog_lba: u32,
        load_rba: u32,
        valid_validation_entry: bool,
    ) {
        write_descriptor(path, 16, 1, |descriptor| {
            write_ascii_field(&mut descriptor[7..39], b"OSFLUX TEST PRIMARY VOLUME");
        });
        write_descriptor(path, 17, 0, |descriptor| {
            write_ascii_field(&mut descriptor[7..39], b"EL TORITO SPECIFICATION");
            descriptor[71..75].copy_from_slice(&catalog_lba.to_le_bytes());
        });
        write_descriptor(path, 18, 255, |_| {});
        write_catalog(path, catalog_lba, load_rba, valid_validation_entry);
        write_at(path, u64::from(load_rba) * ISO_SECTOR_SIZE, &[0xF4; 2048]);
    }

    fn write_catalog(
        path: &std::path::Path,
        catalog_lba: u32,
        load_rba: u32,
        valid_validation_entry: bool,
    ) {
        let mut validation = [0u8; 32];
        validation[0] = 0x01;
        validation[1] = 0xEF;
        validation[30] = 0x55;
        validation[31] = 0xAA;
        if valid_validation_entry {
            let checksum = validation
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap()))
                .fold(0u16, u16::wrapping_add);
            validation[28..30].copy_from_slice(&0u16.wrapping_sub(checksum).to_le_bytes());
        }

        let mut default_entry = [0u8; 32];
        default_entry[0] = 0x88;
        default_entry[1] = 0x00;
        default_entry[6..8].copy_from_slice(&4u16.to_le_bytes());
        default_entry[8..12].copy_from_slice(&load_rba.to_le_bytes());

        write_at(path, u64::from(catalog_lba) * ISO_SECTOR_SIZE, &validation);
        write_at(
            path,
            (u64::from(catalog_lba) * ISO_SECTOR_SIZE) + 32,
            &default_entry,
        );
    }

    fn write_descriptor<F>(path: &std::path::Path, sector: u64, descriptor_type: u8, build: F)
    where
        F: FnOnce(&mut [u8; ISO_SECTOR_SIZE as usize]),
    {
        let mut descriptor = [0u8; ISO_SECTOR_SIZE as usize];
        descriptor[0] = descriptor_type;
        descriptor[1..6].copy_from_slice(b"CD001");
        descriptor[6] = 1;
        build(&mut descriptor);
        write_at(path, sector * ISO_SECTOR_SIZE, &descriptor);
    }

    fn write_ascii_field(field: &mut [u8], value: &[u8]) {
        field.fill(b' ');
        field[..value.len()].copy_from_slice(value);
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
