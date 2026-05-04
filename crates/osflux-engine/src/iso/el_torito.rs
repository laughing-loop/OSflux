use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::Path;

const ISO_SECTOR_SIZE: u64 = 2048;
const VOLUME_DESCRIPTOR_START_SECTOR: u64 = 16;
const VOLUME_DESCRIPTOR_SIZE: usize = 2048;
const VOLUME_DESCRIPTOR_IDENTIFIER: &[u8; 5] = b"CD001";
const BOOT_RECORD_DESCRIPTOR_TYPE: u8 = 0;
const TERMINATOR_DESCRIPTOR_TYPE: u8 = 255;
const EL_TORITO_SYSTEM_ID: &[u8] = b"EL TORITO SPECIFICATION";
const BOOT_SYSTEM_ID_RANGE: std::ops::Range<usize> = 7..39;
const BOOT_CATALOG_LBA_RANGE: std::ops::Range<usize> = 71..75;
const CATALOG_ENTRY_SIZE: usize = 32;
const VALIDATION_ENTRY_HEADER_ID: u8 = 0x01;
const VALIDATION_ENTRY_KEY_1: u8 = 0x55;
const VALIDATION_ENTRY_KEY_2: u8 = 0xAA;
const BOOTABLE_INDICATOR: u8 = 0x88;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoBootMetadata {
    pub el_torito: Option<ElToritoBootInfo>,
}

impl IsoBootMetadata {
    pub fn is_bootable(&self) -> bool {
        self.el_torito
            .as_ref()
            .is_some_and(|info| info.default_entry_bootable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElToritoBootInfo {
    pub catalog_lba: u32,
    pub platform: BootCatalogPlatform,
    pub validation_entry_valid: bool,
    pub default_entry_bootable: bool,
    pub emulation: BootEmulation,
    pub load_segment: u16,
    pub system_type: u8,
    pub sector_count: u16,
    pub load_rba: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootCatalogPlatform {
    X86,
    PowerPc,
    Mac,
    Efi,
    Unknown(u8),
}

impl BootCatalogPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::X86 => "BIOS/x86",
            Self::PowerPc => "PowerPC",
            Self::Mac => "Mac",
            Self::Efi => "UEFI",
            Self::Unknown(_) => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootEmulation {
    NoEmulation,
    Floppy12Mb,
    Floppy144Mb,
    Floppy288Mb,
    HardDisk,
    Unknown(u8),
}

impl BootEmulation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoEmulation => "no-emulation",
            Self::Floppy12Mb => "1.2MB floppy",
            Self::Floppy144Mb => "1.44MB floppy",
            Self::Floppy288Mb => "2.88MB floppy",
            Self::HardDisk => "hard-disk",
            Self::Unknown(_) => "unknown",
        }
    }
}

pub fn inspect_boot_metadata(path: impl AsRef<Path>) -> std::io::Result<IsoBootMetadata> {
    let mut file = File::open(path)?;
    let catalog_lba = find_el_torito_catalog_lba(&mut file)?;
    let el_torito = match catalog_lba {
        Some(catalog_lba) => Some(read_boot_catalog(&mut file, catalog_lba)?),
        None => None,
    };

    Ok(IsoBootMetadata { el_torito })
}

fn find_el_torito_catalog_lba(file: &mut File) -> std::io::Result<Option<u32>> {
    for sector in VOLUME_DESCRIPTOR_START_SECTOR.. {
        let Some(descriptor) = read_sector_optional(file, sector)? else {
            return Ok(None);
        };

        if !is_volume_descriptor(&descriptor) {
            continue;
        }

        if descriptor[0] == TERMINATOR_DESCRIPTOR_TYPE {
            return Ok(None);
        }

        if descriptor[0] != BOOT_RECORD_DESCRIPTOR_TYPE {
            continue;
        }

        let boot_system_id = trim_ascii(&descriptor[BOOT_SYSTEM_ID_RANGE]);
        if boot_system_id.as_bytes() != EL_TORITO_SYSTEM_ID {
            continue;
        }

        return Ok(Some(u32::from_le_bytes(
            descriptor[BOOT_CATALOG_LBA_RANGE].try_into().unwrap(),
        )));
    }

    Ok(None)
}

fn read_boot_catalog(file: &mut File, catalog_lba: u32) -> std::io::Result<ElToritoBootInfo> {
    let catalog_offset = u64::from(catalog_lba) * ISO_SECTOR_SIZE;
    let validation_entry: [u8; CATALOG_ENTRY_SIZE] = read_exact_at(file, catalog_offset)?;
    let default_entry: [u8; CATALOG_ENTRY_SIZE] =
        read_exact_at(file, catalog_offset + CATALOG_ENTRY_SIZE as u64)?;

    Ok(ElToritoBootInfo {
        catalog_lba,
        platform: platform_from_id(validation_entry[1]),
        validation_entry_valid: is_validation_entry_valid(&validation_entry),
        default_entry_bootable: default_entry[0] == BOOTABLE_INDICATOR,
        emulation: emulation_from_id(default_entry[1]),
        load_segment: u16::from_le_bytes(default_entry[2..4].try_into().unwrap()),
        system_type: default_entry[4],
        sector_count: u16::from_le_bytes(default_entry[6..8].try_into().unwrap()),
        load_rba: u32::from_le_bytes(default_entry[8..12].try_into().unwrap()),
    })
}

fn is_volume_descriptor(descriptor: &[u8; VOLUME_DESCRIPTOR_SIZE]) -> bool {
    &descriptor[1..6] == VOLUME_DESCRIPTOR_IDENTIFIER && descriptor[6] == 1
}

fn is_validation_entry_valid(entry: &[u8; CATALOG_ENTRY_SIZE]) -> bool {
    entry[0] == VALIDATION_ENTRY_HEADER_ID
        && entry[30] == VALIDATION_ENTRY_KEY_1
        && entry[31] == VALIDATION_ENTRY_KEY_2
        && checksum_is_zero(entry)
}

fn checksum_is_zero(entry: &[u8; CATALOG_ENTRY_SIZE]) -> bool {
    entry
        .chunks_exact(2)
        .map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap()))
        .fold(0u16, u16::wrapping_add)
        == 0
}

fn platform_from_id(value: u8) -> BootCatalogPlatform {
    match value {
        0x00 => BootCatalogPlatform::X86,
        0x01 => BootCatalogPlatform::PowerPc,
        0x02 => BootCatalogPlatform::Mac,
        0xEF => BootCatalogPlatform::Efi,
        value => BootCatalogPlatform::Unknown(value),
    }
}

fn emulation_from_id(value: u8) -> BootEmulation {
    match value {
        0x00 => BootEmulation::NoEmulation,
        0x01 => BootEmulation::Floppy12Mb,
        0x02 => BootEmulation::Floppy144Mb,
        0x03 => BootEmulation::Floppy288Mb,
        0x04 => BootEmulation::HardDisk,
        value => BootEmulation::Unknown(value),
    }
}

fn trim_ascii(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_matches(char::from(0))
        .trim()
        .to_string()
}

fn read_sector_optional(
    file: &mut File,
    sector: u64,
) -> std::io::Result<Option<[u8; VOLUME_DESCRIPTOR_SIZE]>> {
    read_exact_at_optional(file, sector * ISO_SECTOR_SIZE)
}

fn read_exact_at<const N: usize>(file: &mut File, offset: u64) -> std::io::Result<[u8; N]> {
    let Some(buffer) = read_exact_at_optional(file, offset)? else {
        return Err(std::io::Error::new(
            ErrorKind::UnexpectedEof,
            "unexpected end of ISO while reading boot catalog",
        ));
    };

    Ok(buffer)
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
        inspect_boot_metadata, BootCatalogPlatform, BootEmulation, CATALOG_ENTRY_SIZE,
        ISO_SECTOR_SIZE,
    };
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::Path;

    #[test]
    fn detects_el_torito_boot_catalog() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        write_bootable_iso(&path, 20, 24);

        let metadata = inspect_boot_metadata(&path).unwrap();

        assert!(metadata.is_bootable());
        let info = metadata.el_torito.as_ref().unwrap();
        assert_eq!(info.catalog_lba, 20);
        assert_eq!(info.platform, BootCatalogPlatform::Efi);
        assert!(info.validation_entry_valid);
        assert!(info.default_entry_bootable);
        assert_eq!(info.emulation, BootEmulation::NoEmulation);
        assert_eq!(info.sector_count, 4);
        assert_eq!(info.load_rba, 24);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn returns_none_when_no_el_torito_record_exists() {
        let path = std::env::temp_dir().join(unique_name("iso"));
        write_primary_descriptor_only(&path);

        let metadata = inspect_boot_metadata(&path).unwrap();

        assert!(metadata.el_torito.is_none());
        assert!(!metadata.is_bootable());

        let _ = fs::remove_file(path);
    }

    pub fn write_bootable_iso(path: &Path, catalog_lba: u32, load_rba: u32) {
        write_primary_descriptor_only(path);
        write_boot_record_descriptor(path, catalog_lba);
        write_catalog(path, catalog_lba, load_rba);
    }

    fn write_primary_descriptor_only(path: &Path) {
        write_descriptor(path, 16, 1, |descriptor| {
            write_ascii_field(&mut descriptor[7..39], b"OSFLUX TEST PRIMARY VOLUME");
        });
        write_descriptor(path, 18, 255, |_| {});
    }

    fn write_boot_record_descriptor(path: &Path, catalog_lba: u32) {
        write_descriptor(path, 17, 0, |descriptor| {
            write_ascii_field(&mut descriptor[7..39], b"EL TORITO SPECIFICATION");
            descriptor[71..75].copy_from_slice(&catalog_lba.to_le_bytes());
        });
    }

    fn write_catalog(path: &Path, catalog_lba: u32, load_rba: u32) {
        let mut validation = [0u8; CATALOG_ENTRY_SIZE];
        validation[0] = 0x01;
        validation[1] = 0xEF;
        validation[30] = 0x55;
        validation[31] = 0xAA;
        let checksum = validation
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes(bytes.try_into().unwrap()))
            .fold(0u16, u16::wrapping_add);
        validation[28..30].copy_from_slice(&0u16.wrapping_sub(checksum).to_le_bytes());

        let mut default_entry = [0u8; CATALOG_ENTRY_SIZE];
        default_entry[0] = 0x88;
        default_entry[1] = 0x00;
        default_entry[2..4].copy_from_slice(&0x7C0u16.to_le_bytes());
        default_entry[6..8].copy_from_slice(&4u16.to_le_bytes());
        default_entry[8..12].copy_from_slice(&load_rba.to_le_bytes());

        write_at(path, u64::from(catalog_lba) * ISO_SECTOR_SIZE, &validation);
        write_at(
            path,
            (u64::from(catalog_lba) * ISO_SECTOR_SIZE) + CATALOG_ENTRY_SIZE as u64,
            &default_entry,
        );
    }

    fn write_descriptor<F>(path: &Path, sector: u64, descriptor_type: u8, build: F)
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

    fn write_ascii_field(field: &mut [u8], value: &[u8]) {
        field.fill(b' ');
        field[..value.len()].copy_from_slice(value);
    }

    fn unique_name(extension: &str) -> String {
        format!(
            "osflux-el-torito-test-{}.{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            extension
        )
    }
}
