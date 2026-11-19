//! What a drive holds right now, read without changing anything on it.
//!
//! A drive is `blank` when its first and last MiB are uniformly empty, and
//! `empty-filesystem` when it holds exactly one FAT32 or exFAT partition whose
//! root folder has nothing in it but the volume label and a
//! `System Volume Information` folder. Only those two states let a write be
//! confirmed with the drive's short identity code instead of the full phrase.
//! Anything else, including a drive OSflux cannot read, needs the full phrase.

use std::collections::HashSet;

use osflux_core::device::{self, DeviceAccess, DeviceRead, PhysicalDevice};

use crate::crc32::crc32;
use crate::json::{optional_number_json, optional_string_json, string_json};

const MIB: u64 = 1024 * 1024;
/// How much of each end of the drive must be empty for it to count as blank.
/// It covers an MBR, a GPT and its backup copy, and the start of any
/// filesystem or ISO.
const EDGE_BYTES: u64 = MIB;
/// FAT allows at most 65,536 entries of 32 bytes in a folder; exFAT root
/// folders this large do not occur on a stick that is meant to be empty.
const MAX_ROOT_FOLDER_BYTES: usize = 2 * MIB as usize;
const ROOT_READ_PIECE_BYTES: u64 = 64 * 1024;
/// Partitions past this count are listed but not looked inside.
const MAX_INSPECTED_PARTITIONS: usize = 8;
const MAX_GPT_TABLE_BYTES: u64 = 4 * MIB;
const SYSTEM_VOLUME_INFORMATION: &str = "System Volume Information";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveContentsState {
    /// The first and last MiB are uniformly empty.
    Blank,
    /// Exactly one FAT32 or exFAT partition with an empty root folder.
    EmptyFilesystem,
    /// Anything else that could be read.
    HasData,
    /// The drive could not be read, so nothing is known about it.
    Unknown,
}

impl DriveContentsState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blank => "blank",
            Self::EmptyFilesystem => "empty-filesystem",
            Self::HasData => "has-data",
            Self::Unknown => "unknown",
        }
    }

    /// Whether a write to this drive may be confirmed with the short code.
    pub fn allows_short_confirmation(self) -> bool {
        matches!(self, Self::Blank | Self::EmptyFilesystem)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionScheme {
    Mbr,
    Gpt,
}

impl PartitionScheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mbr => "mbr",
            Self::Gpt => "gpt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemKind {
    Fat12,
    Fat16,
    Fat32,
    ExFat,
    Ntfs,
}

impl FilesystemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fat12 => "FAT12",
            Self::Fat16 => "FAT16",
            Self::Fat32 => "FAT32",
            Self::ExFat => "exFAT",
            Self::Ntfs => "NTFS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrivePartition {
    /// 1-based, in partition table order.
    pub number: u32,
    pub start_bytes: u64,
    pub size_bytes: u64,
    /// `None` when the partition holds no filesystem OSflux recognises.
    pub filesystem: Option<FilesystemKind>,
    pub label: Option<String>,
    /// From the filesystem's own bookkeeping, when it keeps one. exFAT only
    /// records a whole percentage, so its value is approximate.
    pub used_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveContents {
    pub state: DriveContentsState,
    /// One line on what was found, for people.
    pub reason: String,
    pub partition_scheme: Option<PartitionScheme>,
    pub partitions: Vec<DrivePartition>,
}

impl DriveContents {
    fn unknown(reason: impl Into<String>) -> Self {
        Self {
            state: DriveContentsState::Unknown,
            reason: reason.into(),
            partition_scheme: None,
            partitions: Vec::new(),
        }
    }

    fn has_data(
        reason: impl Into<String>,
        partition_scheme: Option<PartitionScheme>,
        partitions: Vec<DrivePartition>,
    ) -> Self {
        Self {
            state: DriveContentsState::HasData,
            reason: reason.into(),
            partition_scheme,
            partitions,
        }
    }

    /// The contents as a JSON object, indented to sit `indent` spaces deep.
    pub fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let partitions = if self.partitions.is_empty() {
            "[]".to_string()
        } else {
            let items = self
                .partitions
                .iter()
                .map(|partition| {
                    format!(
                        concat!(
                            "{pad}    {{\n",
                            "{pad}      \"number\": {},\n",
                            "{pad}      \"startBytes\": {},\n",
                            "{pad}      \"sizeBytes\": {},\n",
                            "{pad}      \"filesystem\": {},\n",
                            "{pad}      \"label\": {},\n",
                            "{pad}      \"usedBytes\": {}\n",
                            "{pad}    }}"
                        ),
                        partition.number,
                        partition.start_bytes,
                        partition.size_bytes,
                        optional_string_json(partition.filesystem.map(FilesystemKind::as_str)),
                        optional_string_json(partition.label.as_deref()),
                        optional_number_json(partition.used_bytes),
                        pad = pad,
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n");
            format!("[\n{items}\n{pad}  ]")
        };

        format!(
            concat!(
                "{{\n",
                "{pad}  \"state\": {},\n",
                "{pad}  \"reason\": {},\n",
                "{pad}  \"partitionScheme\": {},\n",
                "{pad}  \"partitions\": {}\n",
                "{pad}}}"
            ),
            string_json(self.state.as_str()),
            string_json(&self.reason),
            optional_string_json(self.partition_scheme.map(PartitionScheme::as_str)),
            partitions,
            pad = pad,
        )
    }
}

/// Opens `device` read-only and inspects it. A drive that cannot be opened
/// or read, including any drive when OSflux is not running as Administrator,
/// comes back as [`DriveContentsState::Unknown`].
pub fn inspect_device_contents(device: &PhysicalDevice) -> DriveContents {
    if device.access == DeviceAccess::MetadataOnly {
        return DriveContents::unknown(
            "OSflux needs Administrator rights to read what the drive holds",
        );
    }

    match device::open_read_only(&device.path) {
        Ok(drive) => inspect_drive_contents(&drive),
        Err(error) => DriveContents::unknown(format!("the drive could not be read: {error}")),
    }
}

/// Inspects a drive that is already open. Nothing is written.
pub fn inspect_drive_contents(drive: &dyn DeviceRead) -> DriveContents {
    let disk = match Disk::new(drive) {
        Ok(disk) => disk,
        Err(reason) => return DriveContents::unknown(reason),
    };

    match inspect(&disk) {
        Ok(contents) => contents,
        Err(Problem::Unreadable(reason)) => {
            DriveContents::unknown(format!("the drive could not be read: {reason}"))
        }
        Err(Problem::NotEligible(reason)) => DriveContents::has_data(reason, None, Vec::new()),
    }
}

/// Why an inspection step stopped: the drive could not be read at all, or
/// what it holds rules out the short confirm.
#[derive(Debug)]
enum Problem {
    Unreadable(String),
    NotEligible(String),
}

fn not_eligible<T>(reason: impl Into<String>) -> Result<T, Problem> {
    Err(Problem::NotEligible(reason.into()))
}

struct Disk<'a> {
    drive: &'a dyn DeviceRead,
    size: u64,
    sector: u64,
    alignment: u64,
}

impl<'a> Disk<'a> {
    fn new(drive: &'a dyn DeviceRead) -> Result<Self, String> {
        let size = drive
            .size_bytes()
            .map_err(|error| format!("the drive size could not be read: {error}"))?;
        let sector = u64::from(
            drive
                .sector_size_bytes()
                .map_err(|error| format!("the sector size could not be read: {error}"))?,
        );
        if !matches!(sector, 512 | 1024 | 2048 | 4096) {
            return Err(format!(
                "the drive reports an unusual sector size of {sector} bytes"
            ));
        }

        Ok(Self {
            drive,
            size,
            sector,
            // Whole 4 KiB pieces suit both 512-byte and 4K-sector drives.
            alignment: sector.max(4096),
        })
    }

    /// Reads `length` bytes at `offset`, widening the read to whole aligned
    /// pieces as raw drives require.
    fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, Problem> {
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= self.size)
            .ok_or_else(|| {
                Problem::NotEligible(format!(
                    "the drive's tables point past its end (byte {offset} + {length})"
                ))
            })?;
        if length == 0 {
            return Ok(Vec::new());
        }

        let start = offset - offset % self.alignment;
        let aligned_end = end
            .div_ceil(self.alignment)
            .saturating_mul(self.alignment)
            .min(self.size);
        let mut buffer = vec![0u8; (aligned_end - start) as usize];
        self.drive
            .read_exact_at(start, &mut buffer)
            .map_err(|error| Problem::Unreadable(error.to_string()))?;

        let skip = (offset - start) as usize;
        buffer.truncate(skip + length as usize);
        buffer.drain(..skip);
        Ok(buffer)
    }

    fn read_u32(&self, offset: u64) -> Result<u32, Problem> {
        let bytes = self.read(offset, 4)?;
        Ok(u32_at(&bytes, 0))
    }
}

fn inspect(disk: &Disk) -> Result<DriveContents, Problem> {
    if disk.size < 2 * EDGE_BYTES {
        return Ok(DriveContents::unknown("the drive is smaller than 2 MiB"));
    }

    let start = disk.read(0, EDGE_BYTES)?;
    let end = disk.read(disk.size - EDGE_BYTES, EDGE_BYTES)?;
    match (is_uniform(&start), is_uniform(&end)) {
        (true, true) => {
            return Ok(DriveContents {
                state: DriveContentsState::Blank,
                reason: "the start and end of the drive are empty".to_string(),
                partition_scheme: None,
                partitions: Vec::new(),
            });
        }
        (true, false) => {
            return Ok(DriveContents::has_data(
                "the start of the drive is empty but its end is not, such as a leftover backup partition table",
                None,
                Vec::new(),
            ));
        }
        _ => {}
    }

    let (scheme, entries) = read_partition_table(disk, &start)?;

    let mut partitions = Vec::with_capacity(entries.len());
    let mut probes = Vec::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        let probe = if entry.extended || position >= MAX_INSPECTED_PARTITIONS {
            FilesystemProbe::default()
        } else {
            probe_partition(disk, entry)?
        };
        partitions.push(DrivePartition {
            number: position as u32 + 1,
            start_bytes: entry.start_bytes,
            size_bytes: entry.size_bytes,
            filesystem: probe.kind,
            label: probe.label.clone(),
            used_bytes: probe.used_bytes,
        });
        probes.push(probe);
    }

    let has_data = |reason: String| {
        Ok(DriveContents::has_data(
            reason,
            Some(scheme),
            partitions.clone(),
        ))
    };

    if entries.iter().any(|entry| entry.extended) {
        return has_data("the drive has an extended partition".to_string());
    }

    match probes.as_slice() {
        [] => has_data("the drive has a partition table but no partitions".to_string()),
        [probe] => {
            let kind = probe.kind;
            match (kind, &probe.root) {
                (
                    Some(kind @ (FilesystemKind::Fat32 | FilesystemKind::ExFat)),
                    RootFolder::Empty,
                ) => Ok(DriveContents {
                    state: DriveContentsState::EmptyFilesystem,
                    reason: format!("the drive has one empty {} partition", kind.as_str()),
                    partition_scheme: Some(scheme),
                    partitions: partitions.clone(),
                }),
                (
                    Some(kind @ (FilesystemKind::Fat32 | FilesystemKind::ExFat)),
                    RootFolder::Holds { first, count },
                ) => has_data(format!(
                    "the {} partition's root folder holds {}",
                    kind.as_str(),
                    describe_entries(first, *count)
                )),
                (Some(_), RootFolder::Unclear(reason)) => has_data(reason.clone()),
                (Some(kind), _) => has_data(format!(
                    "the partition holds {}, not FAT32 or exFAT",
                    kind.as_str()
                )),
                (None, _) => {
                    has_data("the partition holds no FAT32 or exFAT filesystem".to_string())
                }
            }
        }
        many => has_data(format!("the drive has {} partitions", many.len())),
    }
}

fn describe_entries(first: &str, count: usize) -> String {
    match count {
        0 | 1 => format!("\"{first}\""),
        2 => format!("\"{first}\" and 1 more entry"),
        _ => format!("\"{first}\" and {} more entries", count - 1),
    }
}

/// True for all zero bytes, or all 0xFF as some erased flash reads.
fn is_uniform(bytes: &[u8]) -> bool {
    match bytes.first() {
        Some(&first @ (0x00 | 0xFF)) => bytes.iter().all(|&byte| byte == first),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy)]
struct PartitionEntry {
    start_bytes: u64,
    size_bytes: u64,
    extended: bool,
}

fn read_partition_table(
    disk: &Disk,
    start: &[u8],
) -> Result<(PartitionScheme, Vec<PartitionEntry>), Problem> {
    let sector0 = &start[..512];

    if let Some(kind) = boot_sector_kind(sector0) {
        return not_eligible(format!(
            "a {} filesystem fills the whole drive, without a partition table",
            kind.as_str()
        ));
    }

    if sector0[510..512] != [0x55, 0xAA] {
        return not_eligible("the start of the drive holds data OSflux does not recognise");
    }

    let mut mbr_entries = Vec::new();
    for slot in 0..4 {
        let entry = &sector0[446 + slot * 16..446 + (slot + 1) * 16];
        let partition_type = entry[4];
        if partition_type == 0 {
            continue;
        }
        if entry[0] != 0x00 && entry[0] != 0x80 {
            return not_eligible("the MBR partition table is damaged");
        }
        mbr_entries.push((
            partition_type,
            u64::from(u32_at(entry, 8)),
            u64::from(u32_at(entry, 12)),
        ));
    }

    let gpt_signature = &start[disk.sector as usize..disk.sector as usize + 8];
    if mbr_entries.iter().any(|(kind, ..)| *kind == 0xEE) {
        let entries = read_gpt(disk, start)?;
        return Ok((PartitionScheme::Gpt, entries));
    }
    if gpt_signature == b"EFI PART" {
        return not_eligible("the drive has both an MBR and a GPT partition table");
    }

    let mut entries = Vec::with_capacity(mbr_entries.len());
    for (partition_type, first_sector, sector_count) in mbr_entries {
        let start_bytes = first_sector * disk.sector;
        let size_bytes = sector_count * disk.sector;
        if first_sector == 0
            || sector_count == 0
            || start_bytes
                .checked_add(size_bytes)
                .is_none_or(|end| end > disk.size)
        {
            return not_eligible("an MBR partition lies outside the drive");
        }
        entries.push(PartitionEntry {
            start_bytes,
            size_bytes,
            extended: matches!(partition_type, 0x05 | 0x0F | 0x85),
        });
    }
    reject_overlaps(&entries)?;

    Ok((PartitionScheme::Mbr, entries))
}

fn read_gpt(disk: &Disk, start: &[u8]) -> Result<Vec<PartitionEntry>, Problem> {
    let damaged = || not_eligible("the GPT partition table is damaged");
    let sector = disk.sector as usize;
    let header = &start[sector..sector * 2];

    if &header[0..8] != b"EFI PART" {
        return damaged();
    }
    let header_size = u32_at(header, 12) as usize;
    if !(92..=sector).contains(&header_size) {
        return damaged();
    }
    let mut checked = header[..header_size].to_vec();
    checked[16..20].fill(0);
    if crc32(&checked) != u32_at(header, 16) || u64_at(header, 24) != 1 {
        return damaged();
    }

    let first_usable = u64_at(header, 40);
    let last_usable = u64_at(header, 48);
    let table_sector = u64_at(header, 72);
    let entry_count = u64::from(u32_at(header, 80));
    let entry_size = u64::from(u32_at(header, 84));
    if entry_size < 128 || entry_size % 8 != 0 || entry_count == 0 {
        return damaged();
    }
    let table_bytes = entry_count * entry_size;
    if table_bytes > MAX_GPT_TABLE_BYTES {
        return damaged();
    }
    let table = disk.read(table_sector * disk.sector, table_bytes)?;
    if crc32(&table) != u32_at(header, 88) {
        return damaged();
    }

    let mut entries = Vec::new();
    for raw in table.chunks_exact(entry_size as usize) {
        if raw[..16].iter().all(|&byte| byte == 0) {
            continue;
        }
        let first = u64_at(raw, 32);
        let last = u64_at(raw, 40);
        if first > last
            || first < first_usable
            || last > last_usable
            || (last + 1).saturating_mul(disk.sector) > disk.size
        {
            return not_eligible("a GPT partition lies outside the drive");
        }
        entries.push(PartitionEntry {
            start_bytes: first * disk.sector,
            size_bytes: (last - first + 1) * disk.sector,
            extended: false,
        });
    }
    reject_overlaps(&entries)?;

    Ok(entries)
}

fn reject_overlaps(entries: &[PartitionEntry]) -> Result<(), Problem> {
    for (index, first) in entries.iter().enumerate() {
        for second in &entries[index + 1..] {
            let first_end = first.start_bytes + first.size_bytes;
            let second_end = second.start_bytes + second.size_bytes;
            if first.start_bytes < second_end && second.start_bytes < first_end {
                return not_eligible("two partitions overlap");
            }
        }
    }
    Ok(())
}

/// The filesystem a boot sector belongs to, judged from the sector alone.
fn boot_sector_kind(sector: &[u8]) -> Option<FilesystemKind> {
    if sector.len() < 512 || sector[510..512] != [0x55, 0xAA] {
        return None;
    }
    if &sector[3..11] == b"EXFAT   " {
        return Some(FilesystemKind::ExFat);
    }
    if &sector[3..11] == b"NTFS    " {
        return Some(FilesystemKind::Ntfs);
    }
    FatBootSector::parse(sector).map(|boot| boot.kind)
}

#[derive(Debug, Default)]
struct FilesystemProbe {
    kind: Option<FilesystemKind>,
    label: Option<String>,
    used_bytes: Option<u64>,
    root: RootFolder,
}

#[derive(Debug, Default)]
enum RootFolder {
    /// Not looked at, because the filesystem is not one the short confirm
    /// accepts.
    #[default]
    NotChecked,
    Empty,
    Holds {
        first: String,
        count: usize,
    },
    /// Could not be judged: damaged, inconsistent or too large.
    Unclear(String),
}

fn probe_partition(disk: &Disk, entry: &PartitionEntry) -> Result<FilesystemProbe, Problem> {
    let boot = disk.read(entry.start_bytes, 512)?;
    match boot_sector_kind(&boot) {
        Some(FilesystemKind::ExFat) => probe_exfat(disk, entry, &boot),
        Some(FilesystemKind::Fat32) => probe_fat32(disk, entry, &boot),
        Some(kind) => Ok(FilesystemProbe {
            kind: Some(kind),
            label: FatBootSector::parse(&boot).and_then(|boot| boot.label),
            used_bytes: None,
            root: RootFolder::NotChecked,
        }),
        None => Ok(FilesystemProbe::default()),
    }
}

/// The fields of a FAT12, FAT16 or FAT32 boot sector that OSflux needs.
#[derive(Debug)]
struct FatBootSector {
    kind: FilesystemKind,
    bytes_per_sector: u64,
    sectors_per_cluster: u64,
    reserved_sectors: u64,
    fat_count: u64,
    fat_sectors: u64,
    total_sectors: u64,
    cluster_count: u64,
    root_cluster: u32,
    fs_info_sector: u64,
    label: Option<String>,
}

impl FatBootSector {
    fn parse(sector: &[u8]) -> Option<Self> {
        if sector.len() < 512 || sector[510..512] != [0x55, 0xAA] {
            return None;
        }
        if sector[0] != 0xEB && sector[0] != 0xE9 {
            return None;
        }

        let bytes_per_sector = u64::from(u16_at(sector, 11));
        let sectors_per_cluster = u64::from(sector[13]);
        let reserved_sectors = u64::from(u16_at(sector, 14));
        let fat_count = u64::from(sector[16]);
        let root_entries = u64::from(u16_at(sector, 17));
        let total_16 = u64::from(u16_at(sector, 19));
        let media = sector[21];
        let fat_sectors_16 = u64::from(u16_at(sector, 22));
        let total_32 = u64::from(u32_at(sector, 32));
        let fat_sectors_32 = u64::from(u32_at(sector, 36));

        if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096)
            || sectors_per_cluster == 0
            || !sectors_per_cluster.is_power_of_two()
            || reserved_sectors == 0
            || !(1..=2).contains(&fat_count)
            || (media != 0xF0 && media < 0xF8)
        {
            return None;
        }

        let total_sectors = if total_16 != 0 { total_16 } else { total_32 };
        let fat_sectors = if fat_sectors_16 != 0 {
            fat_sectors_16
        } else {
            fat_sectors_32
        };
        if total_sectors == 0 || fat_sectors == 0 {
            return None;
        }

        let root_sectors = (root_entries * 32).div_ceil(bytes_per_sector);
        let metadata_sectors = reserved_sectors + fat_count * fat_sectors + root_sectors;
        if metadata_sectors >= total_sectors {
            return None;
        }
        let cluster_count = (total_sectors - metadata_sectors) / sectors_per_cluster;

        // Microsoft's rule: the cluster count alone decides the FAT type.
        let kind = if cluster_count < 4085 {
            FilesystemKind::Fat12
        } else if cluster_count < 65525 {
            FilesystemKind::Fat16
        } else {
            FilesystemKind::Fat32
        };
        if kind == FilesystemKind::Fat32 && (root_entries != 0 || fat_sectors_16 != 0) {
            return None;
        }

        let (signature_offset, label_offset) = if kind == FilesystemKind::Fat32 {
            (66, 71)
        } else {
            (38, 43)
        };
        let label = (sector[signature_offset] == 0x29)
            .then(|| short_text(&sector[label_offset..label_offset + 11]))
            .flatten()
            .filter(|label| label != "NO NAME");

        Some(Self {
            kind,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            fat_count,
            fat_sectors,
            total_sectors,
            cluster_count,
            root_cluster: u32_at(sector, 44),
            fs_info_sector: u64::from(u16_at(sector, 48)),
            label,
        })
    }
}

/// Where a filesystem keeps its cluster chain and its clusters.
struct ClusterLayout {
    fat_offset: u64,
    heap_offset: u64,
    cluster_bytes: u64,
    last_cluster: u64,
    entry_mask: u32,
    end_of_chain: u32,
    bad_cluster: u32,
}

impl ClusterLayout {
    fn cluster_offset(&self, cluster: u64) -> u64 {
        self.heap_offset + (cluster - 2) * self.cluster_bytes
    }

    /// Reads a folder by following its cluster chain, stopping at the first
    /// end-of-folder entry.
    fn read_folder(&self, disk: &Disk, first_cluster: u32) -> Result<Vec<u8>, Problem> {
        let mut data = Vec::new();
        let mut seen = HashSet::new();
        let mut cluster = u64::from(first_cluster);

        loop {
            if cluster < 2 || cluster > self.last_cluster {
                return not_eligible(format!(
                    "the root folder points to cluster {cluster}, outside the filesystem"
                ));
            }
            if !seen.insert(cluster) {
                return not_eligible("the root folder's cluster chain loops back on itself");
            }

            let base = self.cluster_offset(cluster);
            let mut within = 0;
            while within < self.cluster_bytes {
                let length = ROOT_READ_PIECE_BYTES.min(self.cluster_bytes - within);
                let piece = disk.read(base + within, length)?;
                let ends = piece.chunks_exact(32).any(|entry| entry[0] == 0);
                data.extend_from_slice(&piece);
                if ends {
                    return Ok(data);
                }
                if data.len() >= MAX_ROOT_FOLDER_BYTES {
                    return not_eligible("the root folder is too large to check");
                }
                within += length;
            }

            let next = disk.read_u32(self.fat_offset + cluster * 4)? & self.entry_mask;
            if next >= self.end_of_chain {
                return Ok(data);
            }
            if next == self.bad_cluster {
                return not_eligible("the root folder uses a cluster marked as bad");
            }
            cluster = u64::from(next);
        }
    }
}

fn probe_fat32(
    disk: &Disk,
    entry: &PartitionEntry,
    boot: &[u8],
) -> Result<FilesystemProbe, Problem> {
    let Some(fat) = FatBootSector::parse(boot) else {
        return Ok(FilesystemProbe::default());
    };
    let mut probe = FilesystemProbe {
        kind: Some(FilesystemKind::Fat32),
        label: fat.label.clone(),
        used_bytes: None,
        root: RootFolder::NotChecked,
    };

    if fat.total_sectors * fat.bytes_per_sector > entry.size_bytes {
        probe.root =
            RootFolder::Unclear("the FAT32 filesystem is larger than its partition".to_string());
        return Ok(probe);
    }

    let cluster_bytes = fat.bytes_per_sector * fat.sectors_per_cluster;
    let layout = ClusterLayout {
        fat_offset: entry.start_bytes + fat.reserved_sectors * fat.bytes_per_sector,
        heap_offset: entry.start_bytes
            + (fat.reserved_sectors + fat.fat_count * fat.fat_sectors) * fat.bytes_per_sector,
        cluster_bytes,
        last_cluster: fat.cluster_count + 1,
        entry_mask: 0x0FFF_FFFF,
        end_of_chain: 0x0FFF_FFF8,
        bad_cluster: 0x0FFF_FFF7,
    };

    probe.used_bytes = fat32_used_bytes(disk, entry, &fat, cluster_bytes)?;

    match layout.read_folder(disk, fat.root_cluster) {
        Ok(folder) => {
            let mut scan = scan_fat_root(&folder);
            // Windows keeps the label it shows in the root folder.
            if let Some(label) = scan.label.take() {
                probe.label = Some(label);
            }
            probe.root = scan.into_root_folder();
        }
        Err(Problem::NotEligible(reason)) => probe.root = RootFolder::Unclear(reason),
        Err(problem) => return Err(problem),
    }

    Ok(probe)
}

/// Used space from the FSInfo sector's free-cluster count, when it holds one.
fn fat32_used_bytes(
    disk: &Disk,
    entry: &PartitionEntry,
    fat: &FatBootSector,
    cluster_bytes: u64,
) -> Result<Option<u64>, Problem> {
    if fat.fs_info_sector == 0 || fat.fs_info_sector >= fat.reserved_sectors {
        return Ok(None);
    }
    let info = disk.read(
        entry.start_bytes + fat.fs_info_sector * fat.bytes_per_sector,
        512,
    )?;
    if u32_at(&info, 0) != 0x4161_5252 || u32_at(&info, 484) != 0x6141_7272 {
        return Ok(None);
    }
    let free = u64::from(u32_at(&info, 488));
    Ok((free <= fat.cluster_count).then(|| (fat.cluster_count - free) * cluster_bytes))
}

#[derive(Debug, Default)]
struct RootScan {
    label: Option<String>,
    entries: Vec<String>,
}

impl RootScan {
    fn into_root_folder(self) -> RootFolder {
        match self.entries.first() {
            None => RootFolder::Empty,
            Some(first) => RootFolder::Holds {
                first: first.clone(),
                count: self.entries.len(),
            },
        }
    }

    fn add(&mut self, name: String, is_folder: bool) {
        if is_folder && name.eq_ignore_ascii_case(SYSTEM_VOLUME_INFORMATION) {
            return;
        }
        self.entries.push(name);
    }
}

/// Lists the live entries of a FAT root folder, putting long names back
/// together and setting aside the volume label.
fn scan_fat_root(folder: &[u8]) -> RootScan {
    let mut scan = RootScan::default();
    let mut long_name = LongName::default();

    for entry in folder.chunks_exact(32) {
        match entry[0] {
            0x00 => break,
            0xE5 => {
                long_name = LongName::default();
                continue;
            }
            _ => {}
        }

        let attributes = entry[11];
        if attributes & 0x3F == 0x0F {
            long_name.add(entry);
            continue;
        }

        let short_name: [u8; 11] = entry[..11].try_into().expect("11 bytes");
        if attributes & 0x08 != 0 && attributes & 0x10 == 0 {
            scan.label = short_text(&short_name);
            long_name = LongName::default();
            continue;
        }

        let name = long_name
            .take(&short_name)
            .unwrap_or_else(|| format_short_name(&short_name, entry[12]));
        scan.add(name, attributes & 0x10 != 0);
    }

    scan
}

/// The pieces of a FAT long file name, collected from the entries that come
/// before its short entry.
#[derive(Default)]
struct LongName {
    units: Vec<u16>,
    parts: u8,
    seen: u32,
    checksum: u8,
}

impl LongName {
    fn add(&mut self, entry: &[u8]) {
        let order = entry[0];
        let sequence = order & 0x1F;
        if order & 0x40 != 0 {
            *self = Self {
                units: vec![0xFFFF; usize::from(sequence) * 13],
                parts: sequence,
                seen: 0,
                checksum: entry[13],
            };
        }
        if sequence == 0 || sequence > self.parts || entry[13] != self.checksum {
            *self = Self::default();
            return;
        }

        let start = usize::from(sequence - 1) * 13;
        let offsets = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        for (position, offset) in offsets.iter().enumerate() {
            self.units[start + position] = u16::from_le_bytes([entry[*offset], entry[offset + 1]]);
        }
        self.seen |= 1 << (sequence - 1);
    }

    /// The long name, if every piece arrived and it belongs to `short_name`.
    fn take(&mut self, short_name: &[u8; 11]) -> Option<String> {
        let complete = self.parts > 0 && self.seen == (1u32 << self.parts) - 1;
        let matches = short_name_checksum(short_name) == self.checksum;
        let units = std::mem::take(&mut self.units);
        *self = Self::default();
        if !complete || !matches {
            return None;
        }
        let end = units
            .iter()
            .position(|&unit| unit == 0x0000 || unit == 0xFFFF)
            .unwrap_or(units.len());
        Some(String::from_utf16_lossy(&units[..end]))
    }
}

fn short_name_checksum(short_name: &[u8; 11]) -> u8 {
    short_name.iter().fold(0u8, |sum, &byte| {
        (sum >> 1).wrapping_add(sum << 7).wrapping_add(byte)
    })
}

fn format_short_name(short_name: &[u8; 11], case_flags: u8) -> String {
    let mut raw = *short_name;
    if raw[0] == 0x05 {
        raw[0] = 0xE5;
    }
    let text = |bytes: &[u8], lower: bool| {
        let value: String = bytes.iter().map(|&byte| char::from(byte)).collect();
        let value = value.trim_end().to_string();
        if lower {
            value.to_lowercase()
        } else {
            value
        }
    };
    let base = text(&raw[..8], case_flags & 0x08 != 0);
    let extension = text(&raw[8..], case_flags & 0x10 != 0);
    if extension.is_empty() {
        base
    } else {
        format!("{base}.{extension}")
    }
}

/// Trims a space-padded label; `None` when nothing is left.
fn short_text(bytes: &[u8]) -> Option<String> {
    let value: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// The fields of an exFAT boot sector that OSflux needs. All offsets are
/// relative to the start of the partition.
struct ExFatBootSector {
    bytes_per_sector: u64,
    cluster_bytes: u64,
    volume_sectors: u64,
    fat_offset: u64,
    heap_offset: u64,
    cluster_count: u64,
    root_cluster: u32,
    percent_in_use: u8,
}

impl ExFatBootSector {
    fn parse(sector: &[u8]) -> Option<Self> {
        if &sector[3..11] != b"EXFAT   " || sector[11..64].iter().any(|&byte| byte != 0) {
            return None;
        }
        let bytes_per_sector_shift = u32::from(sector[108]);
        let sectors_per_cluster_shift = u32::from(sector[109]);
        let fat_count = u64::from(sector[110]);
        if !(9..=12).contains(&bytes_per_sector_shift)
            || bytes_per_sector_shift + sectors_per_cluster_shift > 25
            || !(1..=2).contains(&fat_count)
        {
            return None;
        }

        let bytes_per_sector = 1u64 << bytes_per_sector_shift;
        let sectors_per_cluster = 1u64 << sectors_per_cluster_shift;
        let volume_sectors = u64_at(sector, 72);
        let fat_sector = u64::from(u32_at(sector, 80));
        let fat_sectors = u64::from(u32_at(sector, 84));
        let heap_sector = u64::from(u32_at(sector, 88));
        let cluster_count = u64::from(u32_at(sector, 92));
        let root_cluster = u32_at(sector, 96);

        if fat_sector < 24
            || fat_sectors == 0
            || heap_sector < fat_sector + fat_sectors * fat_count
            || cluster_count == 0
            || u64::from(root_cluster) < 2
            || u64::from(root_cluster) > cluster_count + 1
            || heap_sector + cluster_count * sectors_per_cluster > volume_sectors
        {
            return None;
        }

        Some(Self {
            bytes_per_sector,
            cluster_bytes: bytes_per_sector * sectors_per_cluster,
            volume_sectors,
            fat_offset: fat_sector * bytes_per_sector,
            heap_offset: heap_sector * bytes_per_sector,
            cluster_count,
            root_cluster,
            percent_in_use: sector[112],
        })
    }
}

fn probe_exfat(
    disk: &Disk,
    entry: &PartitionEntry,
    boot: &[u8],
) -> Result<FilesystemProbe, Problem> {
    let mut probe = FilesystemProbe {
        kind: Some(FilesystemKind::ExFat),
        ..FilesystemProbe::default()
    };
    let Some(exfat) = ExFatBootSector::parse(boot) else {
        probe.root = RootFolder::Unclear("the exFAT boot sector is damaged".to_string());
        return Ok(probe);
    };

    if exfat.volume_sectors * exfat.bytes_per_sector > entry.size_bytes {
        probe.root =
            RootFolder::Unclear("the exFAT filesystem is larger than its partition".to_string());
        return Ok(probe);
    }

    let heap_bytes = exfat.cluster_count * exfat.cluster_bytes;
    probe.used_bytes =
        (exfat.percent_in_use <= 100).then(|| heap_bytes * u64::from(exfat.percent_in_use) / 100);

    let layout = ClusterLayout {
        fat_offset: entry.start_bytes + exfat.fat_offset,
        heap_offset: entry.start_bytes + exfat.heap_offset,
        cluster_bytes: exfat.cluster_bytes,
        last_cluster: exfat.cluster_count + 1,
        entry_mask: 0xFFFF_FFFF,
        end_of_chain: 0xFFFF_FFF8,
        bad_cluster: 0xFFFF_FFF7,
    };

    match layout.read_folder(disk, exfat.root_cluster) {
        Ok(folder) => {
            let scan = scan_exfat_root(&folder);
            probe.label = scan.label.clone();
            probe.root = scan.into_root_folder();
        }
        Err(Problem::NotEligible(reason)) => probe.root = RootFolder::Unclear(reason),
        Err(problem) => return Err(problem),
    }

    Ok(probe)
}

const EXFAT_END_OF_FOLDER: u8 = 0x00;
const EXFAT_IN_USE: u8 = 0x80;
const EXFAT_ALLOCATION_BITMAP: u8 = 0x81;
const EXFAT_UPCASE_TABLE: u8 = 0x82;
const EXFAT_VOLUME_LABEL: u8 = 0x83;
const EXFAT_FILE: u8 = 0x85;
const EXFAT_VOLUME_GUID: u8 = 0xA0;
const EXFAT_TEXFAT_PADDING: u8 = 0xA1;
const EXFAT_STREAM_EXTENSION: u8 = 0xC0;
const EXFAT_FILE_NAME: u8 = 0xC1;

/// Lists the live entries of an exFAT root folder. The allocation bitmap,
/// upcase table, volume label and GUID belong to the filesystem itself;
/// anything else in use counts as content.
fn scan_exfat_root(folder: &[u8]) -> RootScan {
    let mut scan = RootScan::default();
    let entries: Vec<&[u8]> = folder.chunks_exact(32).collect();
    let mut index = 0;

    while index < entries.len() {
        let entry = entries[index];
        let entry_type = entry[0];
        index += 1;

        if entry_type == EXFAT_END_OF_FOLDER {
            break;
        }
        if entry_type & EXFAT_IN_USE == 0 {
            continue;
        }

        match entry_type {
            EXFAT_ALLOCATION_BITMAP
            | EXFAT_UPCASE_TABLE
            | EXFAT_VOLUME_GUID
            | EXFAT_TEXFAT_PADDING => {}
            EXFAT_VOLUME_LABEL => {
                let characters = usize::from(entry[1]).min(11);
                let units: Vec<u16> = entry[2..2 + characters * 2]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                let label = String::from_utf16_lossy(&units);
                scan.label = (!label.trim().is_empty()).then(|| label.trim().to_string());
            }
            EXFAT_FILE => {
                let secondary_count = usize::from(entry[1]);
                let is_folder = u16::from_le_bytes([entry[4], entry[5]]) & 0x10 != 0;
                let set = &entries[index..(index + secondary_count).min(entries.len())];
                index += secondary_count;
                match exfat_file_name(set, secondary_count) {
                    Some(name) => scan.add(name, is_folder),
                    None => scan
                        .entries
                        .push("an entry OSflux could not read".to_string()),
                }
            }
            other => scan
                .entries
                .push(format!("an unrecognised entry of type 0x{other:02X}")),
        }
    }

    scan
}

/// Puts a file's name back together from its stream extension and name
/// entries; `None` when the set is incomplete or out of order.
fn exfat_file_name(set: &[&[u8]], secondary_count: usize) -> Option<String> {
    if set.len() != secondary_count || secondary_count < 2 {
        return None;
    }
    let stream = set[0];
    if stream[0] != EXFAT_STREAM_EXTENSION {
        return None;
    }
    let name_length = usize::from(stream[3]);

    let mut units = Vec::with_capacity(name_length);
    for name_entry in &set[1..] {
        if name_entry[0] != EXFAT_FILE_NAME {
            break;
        }
        units.extend(
            name_entry[2..32]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
        );
    }
    if units.len() < name_length || name_length == 0 {
        return None;
    }
    units.truncate(name_length);
    Some(String::from_utf16_lossy(&units))
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("8 bytes"))
}

#[cfg(test)]
pub(crate) mod test_drives {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use osflux_core::device::{DeviceError, DeviceRead, DeviceResult};

    /// A drive kept as its non-empty 512-byte sectors, loaded from the
    /// fixtures that `tests/fixtures/drives/make-fixtures.sh` builds with
    /// the standard Linux tools.
    pub(crate) struct SparseDrive {
        size: u64,
        fill: u8,
        sectors: RefCell<BTreeMap<u64, Vec<u8>>>,
        pub(crate) fail_reads: bool,
    }

    impl SparseDrive {
        pub(crate) fn empty(size: u64, fill: u8) -> Self {
            Self {
                size,
                fill,
                sectors: RefCell::new(BTreeMap::new()),
                fail_reads: false,
            }
        }

        pub(crate) fn from_fixture(text: &str) -> Self {
            let mut lines = text.lines();
            assert_eq!(lines.next(), Some("osflux-sparse-image 1"));
            let size = lines
                .next()
                .and_then(|line| line.strip_prefix("size "))
                .and_then(|value| value.parse().ok())
                .expect("size line");
            let drive = Self::empty(size, 0);
            for line in lines {
                let (offset, hex) = line.split_once(' ').expect("offset and data");
                let bytes: Vec<u8> = (0..hex.len())
                    .step_by(2)
                    .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
                    .collect();
                drive.write(offset.parse().expect("offset"), &bytes);
            }
            drive
        }

        pub(crate) fn write(&self, offset: u64, bytes: &[u8]) {
            for (index, byte) in bytes.iter().enumerate() {
                let position = offset + index as u64;
                let sector = position - position % 512;
                let mut sectors = self.sectors.borrow_mut();
                let data = sectors
                    .entry(sector)
                    .or_insert_with(|| vec![self.fill; 512]);
                data[(position - sector) as usize] = *byte;
            }
        }

        pub(crate) fn read(&self, offset: u64, length: usize) -> Vec<u8> {
            let mut buffer = vec![0u8; length];
            self.read_exact_at(offset, &mut buffer).expect("in range");
            buffer
        }
    }

    impl DeviceRead for SparseDrive {
        fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> DeviceResult<()> {
            if self.fail_reads {
                return Err(DeviceError::Io {
                    operation: "read_at",
                    message: "the device is not ready".to_string(),
                });
            }
            assert_eq!(offset % 512, 0, "reads must start on a sector");
            assert_eq!(buffer.len() % 512, 0, "reads must be whole sectors");
            assert!(
                offset + buffer.len() as u64 <= self.size,
                "read past the end"
            );
            buffer.fill(self.fill);
            let sectors = self.sectors.borrow();
            for (sector, data) in sectors.range(offset..offset + buffer.len() as u64) {
                let start = (sector - offset) as usize;
                buffer[start..start + 512].copy_from_slice(data);
            }
            Ok(())
        }

        fn size_bytes(&self) -> DeviceResult<u64> {
            Ok(self.size)
        }

        fn sector_size_bytes(&self) -> DeviceResult<u32> {
            Ok(512)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_drives::SparseDrive;
    use super::{
        inspect_drive_contents, u32_at, DriveContentsState, FilesystemKind, PartitionScheme, MIB,
    };

    const GIB: u64 = 1024 * MIB;

    macro_rules! fixture {
        ($name:literal) => {
            SparseDrive::from_fixture(include_str!(concat!(
                "../tests/fixtures/drives/",
                $name,
                ".sparse"
            )))
        };
    }

    #[test]
    fn an_all_zero_drive_is_blank() {
        let contents = inspect_drive_contents(&SparseDrive::empty(GIB, 0x00));

        assert_eq!(contents.state, DriveContentsState::Blank);
        assert!(contents.partitions.is_empty());
    }

    #[test]
    fn erased_flash_that_reads_as_0xff_is_blank() {
        let contents = inspect_drive_contents(&SparseDrive::empty(GIB, 0xFF));

        assert_eq!(contents.state, DriveContentsState::Blank);
    }

    #[test]
    fn a_leftover_backup_gpt_at_the_end_is_not_blank() {
        let drive = SparseDrive::empty(GIB, 0);
        drive.write(GIB - 512, b"EFI PART");

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert!(contents.reason.contains("end"), "{}", contents.reason);
    }

    #[test]
    fn one_byte_in_the_first_mib_is_enough_to_not_be_blank() {
        let drive = SparseDrive::empty(GIB, 0);
        drive.write(MIB - 1, &[1]);

        assert_eq!(
            inspect_drive_contents(&drive).state,
            DriveContentsState::HasData
        );
    }

    #[test]
    fn a_new_fat32_stick_is_an_empty_filesystem() {
        let contents = inspect_drive_contents(&fixture!("mbr-fat32-empty"));

        assert_eq!(contents.state, DriveContentsState::EmptyFilesystem);
        assert_eq!(contents.reason, "the drive has one empty FAT32 partition");
        assert_eq!(contents.partition_scheme, Some(PartitionScheme::Mbr));
        assert_eq!(contents.partitions.len(), 1);
        let partition = &contents.partitions[0];
        assert_eq!(partition.number, 1);
        assert_eq!(partition.start_bytes, MIB);
        assert_eq!(partition.size_bytes, GIB - MIB);
        assert_eq!(partition.filesystem, Some(FilesystemKind::Fat32));
        assert_eq!(partition.label.as_deref(), Some("KINGSTON"));
        assert!(partition.used_bytes.is_some_and(|used| used < MIB));
    }

    #[test]
    fn system_volume_information_does_not_count_as_content() {
        let contents = inspect_drive_contents(&fixture!("mbr-fat32-svi"));

        assert_eq!(contents.state, DriveContentsState::EmptyFilesystem);
    }

    #[test]
    fn deleted_files_do_not_count_as_content() {
        let contents = inspect_drive_contents(&fixture!("mbr-fat32-deleted"));

        assert_eq!(contents.state, DriveContentsState::EmptyFilesystem);
    }

    #[test]
    fn a_file_in_the_root_folder_is_content_and_is_named() {
        let contents = inspect_drive_contents(&fixture!("mbr-fat32-file"));

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the FAT32 partition's root folder holds \"Holiday Photo.jpg\""
        );
        assert_eq!(contents.partitions.len(), 1);
    }

    #[test]
    fn a_recycle_bin_counts_as_content() {
        let contents = inspect_drive_contents(&fixture!("mbr-fat32-recycle-bin"));

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert!(
            contents.reason.contains("$RECYCLE.BIN"),
            "{}",
            contents.reason
        );
    }

    #[test]
    fn a_system_volume_information_file_is_not_the_folder() {
        let drive = fixture!("mbr-fat32-svi");
        let entry = find_fat_short_entry(&drive, b"SYSTEM~1   ");
        // Clear the folder attribute, leaving a file with the same name.
        let sector = entry - entry % 512;
        let attributes = drive.read(sector, 512)[(entry - sector) as usize + 11] & !0x10;
        drive.write(entry + 11, &[attributes]);

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert!(contents.reason.contains(super::SYSTEM_VOLUME_INFORMATION));
    }

    #[test]
    fn an_empty_fat32_partition_on_gpt_is_an_empty_filesystem() {
        let contents = inspect_drive_contents(&fixture!("gpt-fat32-empty"));

        assert_eq!(contents.state, DriveContentsState::EmptyFilesystem);
        assert_eq!(contents.partition_scheme, Some(PartitionScheme::Gpt));
        assert_eq!(contents.partitions[0].label.as_deref(), Some("GPTSTICK"));
    }

    #[test]
    fn a_gpt_with_a_bad_checksum_is_not_trusted() {
        let drive = fixture!("gpt-fat32-empty");
        // Change one byte of the partition table without fixing its CRC.
        let entry_name = 1024 + 56;
        let byte = drive.read(1024, 512)[56];
        drive.write(entry_name, &[byte ^ 0x01]);

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(contents.reason, "the GPT partition table is damaged");
    }

    #[test]
    fn a_new_exfat_stick_is_an_empty_filesystem() {
        let contents = inspect_drive_contents(&fixture!("mbr-exfat-empty"));

        assert_eq!(contents.state, DriveContentsState::EmptyFilesystem);
        assert_eq!(contents.reason, "the drive has one empty exFAT partition");
        let partition = &contents.partitions[0];
        assert_eq!(partition.filesystem, Some(FilesystemKind::ExFat));
        assert_eq!(partition.label.as_deref(), Some("SANDISK"));
    }

    #[test]
    fn an_exfat_system_volume_information_folder_does_not_count() {
        let drive = fixture!("mbr-exfat-empty");
        add_exfat_entry(&drive, "System Volume Information", true);

        assert_eq!(
            inspect_drive_contents(&drive).state,
            DriveContentsState::EmptyFilesystem
        );
    }

    #[test]
    fn an_exfat_file_counts_as_content_and_is_named() {
        let drive = fixture!("mbr-exfat-empty");
        add_exfat_entry(&drive, "Holiday Photo.jpg", false);

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the exFAT partition's root folder holds \"Holiday Photo.jpg\""
        );
    }

    #[test]
    fn an_exfat_system_volume_information_file_counts_as_content() {
        let drive = fixture!("mbr-exfat-empty");
        add_exfat_entry(&drive, "System Volume Information", false);

        assert_eq!(
            inspect_drive_contents(&drive).state,
            DriveContentsState::HasData
        );
    }

    #[test]
    fn two_partitions_need_the_full_phrase() {
        let contents = inspect_drive_contents(&fixture!("mbr-two-fat32"));

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(contents.reason, "the drive has 2 partitions");
        let labels: Vec<_> = contents
            .partitions
            .iter()
            .map(|partition| partition.label.as_deref())
            .collect();
        assert_eq!(labels, [Some("FIRST"), Some("SECOND")]);
    }

    #[test]
    fn a_filesystem_without_a_partition_table_needs_the_full_phrase() {
        let contents = inspect_drive_contents(&fixture!("superfloppy-fat32"));

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "a FAT32 filesystem fills the whole drive, without a partition table"
        );
    }

    #[test]
    fn fat16_is_not_accepted() {
        let contents = inspect_drive_contents(&fixture!("mbr-fat16-empty"));

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the partition holds FAT16, not FAT32 or exFAT"
        );
        assert_eq!(contents.partitions[0].label.as_deref(), Some("SMALL"));
    }

    #[test]
    fn ntfs_is_not_accepted() {
        let drive = fixture!("mbr-fat32-empty");
        drive.write(MIB + 3, b"NTFS    ");

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the partition holds NTFS, not FAT32 or exFAT"
        );
    }

    #[test]
    fn an_unformatted_partition_is_not_accepted() {
        let drive = fixture!("mbr-fat32-empty");
        drive.write(MIB + 510, &[0, 0]);

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the partition holds no FAT32 or exFAT filesystem"
        );
    }

    #[test]
    fn an_extended_partition_needs_the_full_phrase() {
        let drive = fixture!("mbr-fat32-empty");
        drive.write(446 + 4, &[0x0F]);

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(contents.reason, "the drive has an extended partition");
    }

    #[test]
    fn a_partition_past_the_end_of_the_drive_is_damage() {
        let drive = fixture!("mbr-fat32-empty");
        drive.write(446 + 12, &u32::MAX.to_le_bytes());

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(contents.reason, "an MBR partition lies outside the drive");
    }

    #[test]
    fn a_filesystem_larger_than_its_partition_is_damage() {
        let drive = fixture!("mbr-fat32-empty");
        let sectors = u32_at(&drive.read(0, 512), 446 + 12);
        drive.write(446 + 12, &(sectors / 2).to_le_bytes());

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the FAT32 filesystem is larger than its partition"
        );
    }

    #[test]
    fn a_root_folder_that_loops_is_damage() {
        let drive = fixture!("mbr-fat32-empty");
        let root_fat_entry = fat32_entry_offset(&drive, 2);
        // Root cluster 2 points to itself instead of ending.
        drive.write(root_fat_entry, &2u32.to_le_bytes());
        // Fill the root cluster so the reader has to follow the chain.
        fill_root_cluster_with_deleted_entries(&drive);

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::HasData);
        assert_eq!(
            contents.reason,
            "the root folder's cluster chain loops back on itself"
        );
    }

    #[test]
    fn a_drive_that_cannot_be_read_is_unknown() {
        let mut drive = fixture!("mbr-fat32-empty");
        drive.fail_reads = true;

        let contents = inspect_drive_contents(&drive);

        assert_eq!(contents.state, DriveContentsState::Unknown);
        assert!(contents.reason.contains("not ready"), "{}", contents.reason);
    }

    #[test]
    fn a_tiny_drive_is_unknown() {
        let contents = inspect_drive_contents(&SparseDrive::empty(MIB, 0));

        assert_eq!(contents.state, DriveContentsState::Unknown);
    }

    #[test]
    fn only_blank_and_empty_filesystem_allow_the_short_confirmation() {
        assert!(DriveContentsState::Blank.allows_short_confirmation());
        assert!(DriveContentsState::EmptyFilesystem.allows_short_confirmation());
        assert!(!DriveContentsState::HasData.allows_short_confirmation());
        assert!(!DriveContentsState::Unknown.allows_short_confirmation());
    }

    #[test]
    fn writes_contents_as_json() {
        let json = inspect_drive_contents(&fixture!("mbr-fat32-empty")).to_json(2);

        assert!(json.contains("\"state\": \"empty-filesystem\""), "{json}");
        assert!(json.contains("\"partitionScheme\": \"mbr\""), "{json}");
        assert!(json.contains("\"filesystem\": \"FAT32\""), "{json}");
        assert!(json.contains("\"label\": \"KINGSTON\""), "{json}");
        assert!(json.contains("\"startBytes\": 1048576"), "{json}");
    }

    // Helpers that reach into the fixtures' FAT32 and exFAT structures.

    struct Fat32Layout {
        fat_offset: u64,
        root_offset: u64,
        cluster_bytes: u64,
    }

    fn fat32_layout(drive: &SparseDrive) -> Fat32Layout {
        let boot = drive.read(MIB, 512);
        let bytes_per_sector = u64::from(u16::from_le_bytes([boot[11], boot[12]]));
        let sectors_per_cluster = u64::from(boot[13]);
        let reserved = u64::from(u16::from_le_bytes([boot[14], boot[15]]));
        let fats = u64::from(boot[16]);
        let fat_sectors = u64::from(u32_at(&boot, 36));
        let root_cluster = u64::from(u32_at(&boot, 44));
        let cluster_bytes = bytes_per_sector * sectors_per_cluster;
        let heap = MIB + (reserved + fats * fat_sectors) * bytes_per_sector;
        Fat32Layout {
            fat_offset: MIB + reserved * bytes_per_sector,
            root_offset: heap + (root_cluster - 2) * cluster_bytes,
            cluster_bytes,
        }
    }

    fn fat32_entry_offset(drive: &SparseDrive, cluster: u64) -> u64 {
        fat32_layout(drive).fat_offset + cluster * 4
    }

    fn find_fat_short_entry(drive: &SparseDrive, short_name: &[u8; 11]) -> u64 {
        let layout = fat32_layout(drive);
        let root = drive.read(layout.root_offset, layout.cluster_bytes as usize);
        let position = root
            .chunks_exact(32)
            .position(|entry| &entry[..11] == short_name)
            .expect("short entry present");
        layout.root_offset + position as u64 * 32
    }

    fn fill_root_cluster_with_deleted_entries(drive: &SparseDrive) {
        let layout = fat32_layout(drive);
        let root = drive.read(layout.root_offset, layout.cluster_bytes as usize);
        for (index, entry) in root.chunks_exact(32).enumerate() {
            if entry[0] == 0 {
                drive.write(layout.root_offset + index as u64 * 32, &[0xE5]);
            }
        }
    }

    /// Adds a file or folder entry set to the exFAT root folder of a fixture,
    /// in the first free slot. The set checksum is left at zero: OSflux does
    /// not rely on it.
    fn add_exfat_entry(drive: &SparseDrive, name: &str, is_folder: bool) {
        let boot = drive.read(MIB, 512);
        let bytes_per_sector = 1u64 << boot[108];
        let cluster_bytes = bytes_per_sector << boot[109];
        let heap = MIB + u64::from(u32_at(&boot, 88)) * bytes_per_sector;
        let root_cluster = u64::from(u32_at(&boot, 96));
        let root_offset = heap + (root_cluster - 2) * cluster_bytes;
        let root = drive.read(root_offset, 4096);
        let free = root
            .chunks_exact(32)
            .position(|entry| entry[0] == 0)
            .expect("free slot") as u64;

        let units: Vec<u16> = name.encode_utf16().collect();
        let name_entries = units.len().div_ceil(15);
        let mut set = Vec::new();

        let mut file = [0u8; 32];
        file[0] = 0x85;
        file[1] = (1 + name_entries) as u8;
        file[4] = if is_folder { 0x10 } else { 0x20 };
        set.extend_from_slice(&file);

        let mut stream = [0u8; 32];
        stream[0] = 0xC0;
        stream[1] = 0x01;
        stream[3] = units.len() as u8;
        set.extend_from_slice(&stream);

        for chunk in units.chunks(15) {
            let mut name_entry = [0u8; 32];
            name_entry[0] = 0xC1;
            for (index, unit) in chunk.iter().enumerate() {
                name_entry[2 + index * 2..4 + index * 2].copy_from_slice(&unit.to_le_bytes());
            }
            set.extend_from_slice(&name_entry);
        }

        drive.write(root_offset + free * 32, &set);
    }
}
