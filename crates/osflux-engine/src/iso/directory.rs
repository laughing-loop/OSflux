use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::Path;

const ISO_SECTOR_SIZE: u64 = 2048;
const VOLUME_DESCRIPTOR_START_SECTOR: u64 = 16;
const VOLUME_DESCRIPTOR_SIZE: usize = 2048;
const VOLUME_DESCRIPTOR_IDENTIFIER: &[u8; 5] = b"CD001";
const MAX_VOLUME_DESCRIPTORS: u64 = 32;
const PRIMARY_VOLUME_DESCRIPTOR_TYPE: u8 = 1;
const TERMINATOR_DESCRIPTOR_TYPE: u8 = 255;
const ROOT_DIRECTORY_RECORD_RANGE: std::ops::Range<usize> = 156..190;
const DIRECTORY_RECORD_MIN_SIZE: usize = 34;
const DIRECTORY_EXTENT_LBA_RANGE: std::ops::Range<usize> = 2..6;
const DIRECTORY_DATA_LENGTH_RANGE: std::ops::Range<usize> = 10..14;
const DIRECTORY_FLAGS_OFFSET: usize = 25;
const DIRECTORY_IDENTIFIER_LENGTH_OFFSET: usize = 32;
const DIRECTORY_IDENTIFIER_OFFSET: usize = 33;
const DIRECTORY_FLAG_DIRECTORY: u8 = 0x02;
const MAX_DIRECTORY_DEPTH: usize = 8;
const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_DIRECTORY_BYTES_PER_EXTENT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoDirectoryMetadata {
    pub root_extent_lba: u32,
    pub root_data_length: u32,
    pub scanned_directories: usize,
    pub entries: Vec<IsoDirectoryEntry>,
    pub boot_assets: Vec<IsoBootAsset>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoDirectoryEntry {
    pub path: String,
    pub extent_lba: u32,
    pub data_length: u32,
    pub is_directory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoBootAsset {
    pub kind: IsoBootAssetKind,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoBootAssetKind {
    BootManager,
    UefiBootloader,
    InstallerImage,
    LinuxBootloader,
    LinuxKernel,
    Initrd,
    LiveFilesystem,
    GrubBootPath,
}

impl IsoBootAssetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BootManager => "boot-manager",
            Self::UefiBootloader => "uefi-bootloader",
            Self::InstallerImage => "installer-image",
            Self::LinuxBootloader => "linux-bootloader",
            Self::LinuxKernel => "linux-kernel",
            Self::Initrd => "initrd",
            Self::LiveFilesystem => "live-filesystem",
            Self::GrubBootPath => "grub-boot-path",
        }
    }
}

pub fn inspect_directory_metadata(
    path: impl AsRef<Path>,
) -> std::io::Result<Option<IsoDirectoryMetadata>> {
    let mut file = File::open(path)?;
    let iso_size = file.metadata()?.len();
    inspect_directory_metadata_from_file(&mut file, iso_size)
}

pub fn inspect_directory_metadata_from_file(
    file: &mut File,
    iso_size: u64,
) -> std::io::Result<Option<IsoDirectoryMetadata>> {
    let Some(root) = find_root_directory_record(file)? else {
        return Ok(None);
    };

    let mut queue = VecDeque::from([QueuedDirectory {
        path: String::new(),
        extent_lba: root.extent_lba,
        data_length: root.data_length,
        depth: 0,
    }]);
    let mut visited = HashSet::new();
    let mut entries = Vec::new();
    let mut boot_assets = Vec::new();
    let mut scanned_directories = 0usize;
    let mut truncated = false;

    while let Some(directory) = queue.pop_front() {
        if scanned_directories >= MAX_DIRECTORY_ENTRIES {
            truncated = true;
            break;
        }

        if !visited.insert((directory.extent_lba, directory.data_length)) {
            continue;
        }

        let records = read_directory_records(file, iso_size, &directory, &mut truncated)?;
        scanned_directories += 1;

        for record in records {
            if entries.len() >= MAX_DIRECTORY_ENTRIES {
                truncated = true;
                break;
            }

            let path = join_iso_path(&directory.path, &record.name);
            let entry = IsoDirectoryEntry {
                path: path.clone(),
                extent_lba: record.extent_lba,
                data_length: record.data_length,
                is_directory: record.is_directory,
            };

            if let Some(kind) = classify_boot_asset(&entry.path, entry.is_directory) {
                boot_assets.push(IsoBootAsset {
                    kind,
                    path: entry.path.clone(),
                });
            }

            if record.is_directory && directory.depth + 1 <= MAX_DIRECTORY_DEPTH {
                queue.push_back(QueuedDirectory {
                    path: entry.path.clone(),
                    extent_lba: record.extent_lba,
                    data_length: record.data_length,
                    depth: directory.depth + 1,
                });
            } else if record.is_directory {
                truncated = true;
            }

            entries.push(entry);
        }
    }

    Ok(Some(IsoDirectoryMetadata {
        root_extent_lba: root.extent_lba,
        root_data_length: root.data_length,
        scanned_directories,
        entries,
        boot_assets,
        truncated,
    }))
}

#[derive(Debug, Clone)]
struct QueuedDirectory {
    path: String,
    extent_lba: u32,
    data_length: u32,
    depth: usize,
}

#[derive(Debug, Clone)]
struct ParsedDirectoryRecord {
    name: String,
    extent_lba: u32,
    data_length: u32,
    is_directory: bool,
}

fn find_root_directory_record(file: &mut File) -> std::io::Result<Option<ParsedDirectoryRecord>> {
    let descriptor_sectors =
        VOLUME_DESCRIPTOR_START_SECTOR..VOLUME_DESCRIPTOR_START_SECTOR + MAX_VOLUME_DESCRIPTORS;

    for sector in descriptor_sectors {
        let Some(descriptor) = read_sector_optional(file, sector)? else {
            return Ok(None);
        };

        // Volume descriptors are recorded in consecutive sectors, so the first
        // sector without the identifier ends the set.
        if !has_volume_descriptor_identifier(&descriptor) {
            return Ok(None);
        }

        if !is_volume_descriptor(&descriptor) {
            continue;
        }

        match descriptor[0] {
            PRIMARY_VOLUME_DESCRIPTOR_TYPE => {
                return Ok(parse_directory_record(
                    &descriptor[ROOT_DIRECTORY_RECORD_RANGE],
                ));
            }
            TERMINATOR_DESCRIPTOR_TYPE => return Ok(None),
            _ => {}
        }
    }

    Ok(None)
}

fn read_directory_records(
    file: &mut File,
    iso_size: u64,
    directory: &QueuedDirectory,
    truncated: &mut bool,
) -> std::io::Result<Vec<ParsedDirectoryRecord>> {
    let Some(start_offset) = u64::from(directory.extent_lba).checked_mul(ISO_SECTOR_SIZE) else {
        *truncated = true;
        return Ok(Vec::new());
    };

    if start_offset >= iso_size {
        *truncated = true;
        return Ok(Vec::new());
    }

    let requested_bytes = u64::from(directory.data_length);
    let readable_bytes = requested_bytes
        .min(MAX_DIRECTORY_BYTES_PER_EXTENT)
        .min(iso_size - start_offset);

    if readable_bytes < requested_bytes {
        *truncated = true;
    }

    let mut buffer = vec![0u8; readable_bytes as usize];
    file.seek(SeekFrom::Start(start_offset))?;
    file.read_exact(&mut buffer)?;

    Ok(parse_directory_records(&buffer))
}

fn parse_directory_records(buffer: &[u8]) -> Vec<ParsedDirectoryRecord> {
    let mut records = Vec::new();
    let mut offset = 0usize;

    while offset < buffer.len() {
        let record_length = buffer[offset] as usize;

        if record_length == 0 {
            offset = next_sector_offset(offset);
            continue;
        }

        let end = offset.saturating_add(record_length);
        if end > buffer.len() {
            break;
        }

        if let Some(record) = parse_directory_record(&buffer[offset..end]) {
            if !matches!(record.name.as_str(), "." | "..") {
                records.push(record);
            }
        }

        offset = end;
    }

    records
}

fn parse_directory_record(bytes: &[u8]) -> Option<ParsedDirectoryRecord> {
    if bytes.len() < DIRECTORY_RECORD_MIN_SIZE {
        return None;
    }

    let record_length = bytes[0] as usize;
    if record_length == 0 || record_length > bytes.len() {
        return None;
    }

    let identifier_length = bytes[DIRECTORY_IDENTIFIER_LENGTH_OFFSET] as usize;
    let identifier_end = DIRECTORY_IDENTIFIER_OFFSET.checked_add(identifier_length)?;
    if identifier_end > record_length || identifier_end > bytes.len() {
        return None;
    }

    let identifier = &bytes[DIRECTORY_IDENTIFIER_OFFSET..identifier_end];
    Some(ParsedDirectoryRecord {
        name: normalize_identifier(identifier),
        extent_lba: u32::from_le_bytes(bytes[DIRECTORY_EXTENT_LBA_RANGE].try_into().unwrap()),
        data_length: u32::from_le_bytes(bytes[DIRECTORY_DATA_LENGTH_RANGE].try_into().unwrap()),
        is_directory: bytes[DIRECTORY_FLAGS_OFFSET] & DIRECTORY_FLAG_DIRECTORY != 0,
    })
}

fn normalize_identifier(identifier: &[u8]) -> String {
    match identifier {
        [0] => return ".".to_string(),
        [1] => return "..".to_string(),
        _ => {}
    }

    let raw = String::from_utf8_lossy(identifier)
        .trim_matches(char::from(0))
        .trim()
        .to_string();
    let without_version = raw.split_once(';').map_or(raw.as_str(), |(name, _)| name);
    without_version.trim_end_matches('.').to_string()
}

fn classify_boot_asset(path: &str, is_directory: bool) -> Option<IsoBootAssetKind> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file_name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());

    if normalized == "bootmgr" || normalized.ends_with("/bootmgr") {
        return Some(IsoBootAssetKind::BootManager);
    }

    if normalized.starts_with("efi/boot/") && file_name.ends_with(".efi") {
        return Some(IsoBootAssetKind::UefiBootloader);
    }

    if normalized.starts_with("efi/microsoft/boot") {
        return Some(IsoBootAssetKind::UefiBootloader);
    }

    if matches!(file_name, "install.wim" | "install.esd") {
        return Some(IsoBootAssetKind::InstallerImage);
    }

    if normalized.contains("isolinux") {
        return Some(IsoBootAssetKind::LinuxBootloader);
    }

    if file_name.starts_with("vmlinuz") || file_name.starts_with("vmlinux") {
        return Some(IsoBootAssetKind::LinuxKernel);
    }

    if file_name.starts_with("initrd") || file_name.starts_with("initramfs") {
        return Some(IsoBootAssetKind::Initrd);
    }

    if file_name.ends_with(".squashfs") || normalized.contains("filesystem.squashfs") {
        return Some(IsoBootAssetKind::LiveFilesystem);
    }

    if is_directory && (normalized == "boot/grub" || normalized.ends_with("/boot/grub")) {
        return Some(IsoBootAssetKind::GrubBootPath);
    }

    if normalized.starts_with("boot/grub/") || normalized.contains("/grub/") {
        return Some(IsoBootAssetKind::GrubBootPath);
    }

    None
}

fn join_iso_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn next_sector_offset(offset: usize) -> usize {
    let sector_size = ISO_SECTOR_SIZE as usize;
    ((offset / sector_size) + 1) * sector_size
}

fn has_volume_descriptor_identifier(descriptor: &[u8; VOLUME_DESCRIPTOR_SIZE]) -> bool {
    &descriptor[1..6] == VOLUME_DESCRIPTOR_IDENTIFIER
}

fn is_volume_descriptor(descriptor: &[u8; VOLUME_DESCRIPTOR_SIZE]) -> bool {
    has_volume_descriptor_identifier(descriptor) && descriptor[6] == 1
}

fn read_sector_optional(
    file: &mut File,
    sector: u64,
) -> std::io::Result<Option<[u8; VOLUME_DESCRIPTOR_SIZE]>> {
    read_exact_at_optional(file, sector * ISO_SECTOR_SIZE)
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

#[cfg(test)]
mod tests {
    use super::{
        inspect_directory_metadata, IsoBootAssetKind, ISO_SECTOR_SIZE, VOLUME_DESCRIPTOR_SIZE,
    };
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::Path;

    #[test]
    fn reads_iso_directory_records_and_boot_assets() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        write_iso_with_boot_paths(&path);

        let metadata = inspect_directory_metadata(&path).unwrap().unwrap();

        assert_eq!(metadata.root_extent_lba, 20);
        assert_eq!(metadata.scanned_directories, 4);
        assert!(!metadata.truncated);
        assert!(metadata.entries.iter().any(|entry| entry.path == "BOOTMGR"));
        assert!(metadata
            .entries
            .iter()
            .any(|entry| entry.path == "EFI/BOOT/BOOTX64.EFI"));
        assert!(metadata
            .entries
            .iter()
            .any(|entry| entry.path == "SOURCES/INSTALL.WIM"));
        assert!(metadata.boot_assets.iter().any(|asset| {
            asset.kind == IsoBootAssetKind::BootManager && asset.path == "BOOTMGR"
        }));
        assert!(metadata.boot_assets.iter().any(|asset| {
            asset.kind == IsoBootAssetKind::UefiBootloader && asset.path == "EFI/BOOT/BOOTX64.EFI"
        }));
        assert!(metadata.boot_assets.iter().any(|asset| {
            asset.kind == IsoBootAssetKind::InstallerImage && asset.path == "SOURCES/INSTALL.WIM"
        }));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn returns_none_when_primary_descriptor_is_missing() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        fs::write(&path, b"not an iso").unwrap();

        let metadata = inspect_directory_metadata(&path).unwrap();

        assert!(metadata.is_none());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn ignores_primary_descriptor_after_descriptor_set_ends() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        write_primary_descriptor(&path, 40, 50);
        write_directory(
            &path,
            50,
            &[
                directory_record(&[0], 50, ISO_SECTOR_SIZE as u32, true),
                directory_record(&[1], 50, ISO_SECTOR_SIZE as u32, true),
                directory_record(b"BOOTMGR;1", 51, 16, false),
            ],
        );

        let metadata = inspect_directory_metadata(&path).unwrap();

        assert!(metadata.is_none());

        let _ = fs::remove_file(path);
    }

    fn write_primary_descriptor(path: &Path, sector: u64, root_extent_lba: u32) {
        let mut primary = [0u8; VOLUME_DESCRIPTOR_SIZE];
        primary[0] = 1;
        primary[1..6].copy_from_slice(b"CD001");
        primary[6] = 1;
        let root_record = directory_record(&[0], root_extent_lba, ISO_SECTOR_SIZE as u32, true);
        primary[156..156 + root_record.len()].copy_from_slice(&root_record);
        write_at(path, sector * ISO_SECTOR_SIZE, &primary);

        let mut terminator = [0u8; VOLUME_DESCRIPTOR_SIZE];
        terminator[0] = 255;
        terminator[1..6].copy_from_slice(b"CD001");
        terminator[6] = 1;
        write_at(path, (sector + 1) * ISO_SECTOR_SIZE, &terminator);
    }

    fn write_iso_with_boot_paths(path: &Path) {
        write_primary_descriptor(path, 16, 20);

        write_directory(
            path,
            20,
            &[
                directory_record(&[0], 20, ISO_SECTOR_SIZE as u32, true),
                directory_record(&[1], 20, ISO_SECTOR_SIZE as u32, true),
                directory_record(b"BOOTMGR;1", 30, 4096, false),
                directory_record(b"EFI", 21, ISO_SECTOR_SIZE as u32, true),
                directory_record(b"SOURCES", 23, ISO_SECTOR_SIZE as u32, true),
            ],
        );
        write_directory(
            path,
            21,
            &[
                directory_record(&[0], 21, ISO_SECTOR_SIZE as u32, true),
                directory_record(&[1], 20, ISO_SECTOR_SIZE as u32, true),
                directory_record(b"BOOT", 22, ISO_SECTOR_SIZE as u32, true),
            ],
        );
        write_directory(
            path,
            22,
            &[
                directory_record(&[0], 22, ISO_SECTOR_SIZE as u32, true),
                directory_record(&[1], 21, ISO_SECTOR_SIZE as u32, true),
                directory_record(b"BOOTX64.EFI;1", 31, 1024, false),
            ],
        );
        write_directory(
            path,
            23,
            &[
                directory_record(&[0], 23, ISO_SECTOR_SIZE as u32, true),
                directory_record(&[1], 20, ISO_SECTOR_SIZE as u32, true),
                directory_record(b"INSTALL.WIM;1", 32, 2048, false),
            ],
        );
    }

    fn write_directory(path: &Path, sector: u64, records: &[Vec<u8>]) {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend_from_slice(record);
        }
        bytes.resize(ISO_SECTOR_SIZE as usize, 0);
        write_at(path, sector * ISO_SECTOR_SIZE, &bytes);
    }

    fn directory_record(
        identifier: &[u8],
        extent_lba: u32,
        data_length: u32,
        is_dir: bool,
    ) -> Vec<u8> {
        let padding = if identifier.len() % 2 == 0 { 1 } else { 0 };
        let record_length = 33 + identifier.len() + padding;
        let mut record = vec![0u8; record_length];
        record[0] = record_length as u8;
        record[2..6].copy_from_slice(&extent_lba.to_le_bytes());
        record[6..10].copy_from_slice(&extent_lba.to_be_bytes());
        record[10..14].copy_from_slice(&data_length.to_le_bytes());
        record[14..18].copy_from_slice(&data_length.to_be_bytes());
        record[25] = if is_dir { 0x02 } else { 0x00 };
        record[32] = identifier.len() as u8;
        record[33..33 + identifier.len()].copy_from_slice(identifier);
        record
    }

    fn write_at(path: &Path, offset: u64, bytes: &[u8]) {
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
            "osflux-iso-directory-test-{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        )
    }
}
