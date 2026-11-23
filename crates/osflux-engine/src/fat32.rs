//! Builds a whole drive image, an MBR with one FAT32 partition holding a
//! tree of files, without writing it anywhere first. The image is generated
//! on demand, a piece at a time, so it can be streamed onto a drive and
//! generated again, byte for byte, to verify what was written.
//!
//! The layout follows Microsoft's FAT32 specification (fatgen103):
//!
//! - The MBR has one active partition of type 0x0C from 1 MiB to the last
//!   whole MiB of the drive. Its boot code only prints that the drive starts
//!   in UEFI mode; there is no BIOS boot path.
//! - Clusters are Microsoft's default size for the volume size. The data
//!   region starts on a 1 MiB boundary, which suits flash memory.
//! - Folders come first, then files, each in one contiguous run, so the
//!   image ends at the last file's last cluster and the free space after it
//!   is never written.
//! - Names that are plain 8.3 names are stored as short names, using the
//!   lowercase flags Windows uses. Other names get long name entries and a
//!   unique `~N` short name.

use std::collections::HashSet;
use std::fmt;
use std::io;

use crate::clock::fat_date_time;

pub const MAX_FAT32_FILE_BYTES: u64 = 0xFFFF_FFFF;

const MIB: u64 = 1024 * 1024;
const PARTITION_START_BYTES: u64 = MIB;
const MIN_CLUSTERS: u64 = 65_525;
const MAX_CLUSTERS: u64 = 0x0FFF_FFF5;
const MAX_CLUSTER_BYTES: u64 = 32 * 1024;
const RESERVED_SECTORS: u64 = 32;
const FAT_COUNT: u64 = 2;
const FS_INFO_SECTOR: u64 = 1;
const BACKUP_BOOT_SECTOR: u64 = 6;
const MAX_FOLDER_ENTRIES: u64 = 65_536;
const MAX_LONG_NAME_UNITS: usize = 255;
const END_OF_CHAIN: u32 = 0x0FFF_FFFF;

const ATTRIBUTE_HIDDEN: u8 = 0x02;
const ATTRIBUTE_VOLUME_LABEL: u8 = 0x08;
const ATTRIBUTE_DIRECTORY: u8 = 0x10;
const ATTRIBUTE_ARCHIVE: u8 = 0x20;
const ATTRIBUTE_LONG_NAME: u8 = 0x0F;
const LOWERCASE_BASE: u8 = 0x08;
const LOWERCASE_EXTENSION: u8 = 0x10;

const UEFI_ONLY_MESSAGE: &[u8] =
    b"This drive starts only in UEFI mode. Restart and choose the UEFI: entry for this drive in the boot menu.\r\n\0";

/// Supplies the bytes of the files placed in the image, by the number each
/// [`FatFile`] carries.
pub trait ContentSource {
    fn read_content(&self, content: usize, offset: u64, buffer: &mut [u8]) -> io::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FatDirectory {
    /// Ignored for the root folder.
    pub name: String,
    pub hidden: bool,
    pub entries: Vec<FatEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FatEntry {
    Directory(FatDirectory),
    File(FatFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FatFile {
    pub name: String,
    pub hidden: bool,
    pub size_bytes: u64,
    /// Passed back to the [`ContentSource`] to read this file's bytes.
    pub content: usize,
}

/// What the image is built for, and the values that must stay the same
/// between writing it and generating it again to verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FatImageOptions {
    pub drive_bytes: u64,
    pub sector_bytes: u32,
    pub label: String,
    pub volume_id: u32,
    pub disk_signature: u32,
    pub unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FatError {
    UnsupportedSectorSize {
        sector_bytes: u32,
    },
    DriveTooSmall {
        drive_bytes: u64,
    },
    DriveTooLarge {
        drive_bytes: u64,
    },
    FileTooLarge {
        path: String,
        size_bytes: u64,
    },
    InvalidName {
        path: String,
        reason: &'static str,
    },
    DuplicateName {
        path: String,
    },
    TooManyEntries {
        path: String,
    },
    DoesNotFit {
        needed_bytes: u64,
        available_bytes: u64,
    },
}

impl fmt::Display for FatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSectorSize { sector_bytes } => write!(
                formatter,
                "the drive uses {sector_bytes}-byte sectors; OSflux builds FAT32 for 512-byte and 4096-byte sectors"
            ),
            Self::DriveTooSmall { drive_bytes } => write!(
                formatter,
                "the drive holds {drive_bytes} bytes, too few for a FAT32 filesystem"
            ),
            Self::DriveTooLarge { drive_bytes } => write!(
                formatter,
                "the drive holds {drive_bytes} bytes, more than an MBR partition table can describe"
            ),
            Self::FileTooLarge { path, size_bytes } => write!(
                formatter,
                "{path} is {size_bytes} bytes, and FAT32 cannot hold a file of 4 GiB or more"
            ),
            Self::InvalidName { path, reason } => {
                write!(formatter, "the name of {path} cannot be stored on FAT32: {reason}")
            }
            Self::DuplicateName { path } => write!(
                formatter,
                "{path} appears twice in its folder when letter case is ignored, as FAT32 does"
            ),
            Self::TooManyEntries { path } => {
                write!(formatter, "the folder {path} holds too many entries for FAT32")
            }
            Self::DoesNotFit {
                needed_bytes,
                available_bytes,
            } => write!(
                formatter,
                "the files need {needed_bytes} bytes on the drive, but the FAT32 partition only has {available_bytes}"
            ),
        }
    }
}

impl std::error::Error for FatError {}

/// One contiguous run of clusters: a folder's rendered entries or a file.
#[derive(Debug, Clone)]
struct Run {
    first_cluster: u64,
    clusters: u64,
    data: RunData,
}

#[derive(Debug, Clone)]
enum RunData {
    Folder(Vec<u8>),
    File { content: usize, size_bytes: u64 },
}

/// A drive image laid out and ready to be generated.
#[derive(Debug, Clone)]
pub struct FatImage {
    sector_bytes: u64,
    partition_sectors: u64,
    sectors_per_cluster: u64,
    reserved_sectors: u64,
    fat_sectors: u64,
    cluster_bytes: u64,
    cluster_count: u64,
    used_clusters: u64,
    runs: Vec<Run>,
    mbr: Vec<u8>,
    boot_sector: Vec<u8>,
    fs_info: Vec<u8>,
    label: String,
}

impl FatImage {
    /// Lays out `root` on a drive described by `options`. Nothing is read or
    /// written yet.
    pub fn build(root: &FatDirectory, options: &FatImageOptions) -> Result<Self, FatError> {
        let sector_bytes = u64::from(options.sector_bytes);
        if !matches!(sector_bytes, 512 | 4096) {
            return Err(FatError::UnsupportedSectorSize {
                sector_bytes: options.sector_bytes,
            });
        }

        let partition_end = options.drive_bytes - options.drive_bytes % MIB;
        if partition_end <= PARTITION_START_BYTES {
            return Err(FatError::DriveTooSmall {
                drive_bytes: options.drive_bytes,
            });
        }
        if partition_end / sector_bytes > u64::from(u32::MAX) {
            return Err(FatError::DriveTooLarge {
                drive_bytes: options.drive_bytes,
            });
        }
        let partition_sectors = (partition_end - PARTITION_START_BYTES) / sector_bytes;
        let geometry =
            choose_geometry(partition_sectors, sector_bytes).ok_or(FatError::DriveTooSmall {
                drive_bytes: options.drive_bytes,
            })?;

        let (date, time) = fat_date_time(options.unix_seconds);
        let label = volume_label(&options.label);
        let folders = plan_folders(root, geometry.cluster_bytes)?;

        let mut runs = Vec::new();
        let mut next_cluster = 2u64;
        for folder in &folders {
            runs.push(Run {
                first_cluster: next_cluster,
                clusters: folder.clusters,
                data: RunData::Folder(Vec::new()),
            });
            next_cluster += folder.clusters;
        }
        let mut file_clusters = Vec::new();
        for folder in &folders {
            for file in &folder.files {
                let clusters = file.size_bytes.div_ceil(geometry.cluster_bytes);
                file_clusters.push(if clusters == 0 { 0 } else { next_cluster });
                if clusters > 0 {
                    runs.push(Run {
                        first_cluster: next_cluster,
                        clusters,
                        data: RunData::File {
                            content: file.content,
                            size_bytes: file.size_bytes,
                        },
                    });
                }
                next_cluster += clusters;
            }
        }
        let used_clusters = next_cluster - 2;
        if used_clusters > geometry.cluster_count {
            return Err(FatError::DoesNotFit {
                needed_bytes: used_clusters * geometry.cluster_bytes,
                available_bytes: geometry.cluster_count * geometry.cluster_bytes,
            });
        }

        // Render every folder now that every first cluster is known.
        let folder_clusters: Vec<u64> = runs[..folders.len()]
            .iter()
            .map(|run| run.first_cluster)
            .collect();
        let mut file_index = 0;
        for (index, folder) in folders.iter().enumerate() {
            let own = folder_clusters[index];
            let parent = folder.parent.map(|parent| folder_clusters[parent]);
            let mut bytes = Vec::with_capacity((folder.clusters * geometry.cluster_bytes) as usize);
            match parent {
                None => bytes.extend(short_entry(
                    &label_bytes(&label),
                    ATTRIBUTE_VOLUME_LABEL,
                    0,
                    0,
                    0,
                    date,
                    time,
                )),
                Some(parent_cluster) => {
                    // ".." of a folder in the root points at cluster 0.
                    let parent_cluster = if parent_cluster == 2 {
                        0
                    } else {
                        parent_cluster
                    };
                    bytes.extend(short_entry(
                        b".          ",
                        ATTRIBUTE_DIRECTORY,
                        0,
                        own,
                        0,
                        date,
                        time,
                    ));
                    bytes.extend(short_entry(
                        b"..         ",
                        ATTRIBUTE_DIRECTORY,
                        0,
                        parent_cluster,
                        0,
                        date,
                        time,
                    ));
                }
            }
            for child in &folder.children {
                let (attributes, first_cluster, size) = match child.kind {
                    ChildKind::Folder(child_folder) => (
                        ATTRIBUTE_DIRECTORY | hidden_attribute(child.hidden),
                        folder_clusters[child_folder],
                        0,
                    ),
                    ChildKind::File(size_bytes) => {
                        let first = file_clusters[file_index];
                        file_index += 1;
                        (
                            ATTRIBUTE_ARCHIVE | hidden_attribute(child.hidden),
                            first,
                            size_bytes,
                        )
                    }
                };
                for long_entry in &child.long_entries {
                    bytes.extend_from_slice(long_entry);
                }
                bytes.extend(short_entry(
                    &child.short_name,
                    attributes,
                    child.case_flags,
                    first_cluster,
                    size as u32,
                    date,
                    time,
                ));
            }
            runs[index].data = RunData::Folder(bytes);
        }

        let image = Self {
            sector_bytes,
            partition_sectors,
            sectors_per_cluster: geometry.sectors_per_cluster,
            reserved_sectors: geometry.reserved_sectors,
            fat_sectors: geometry.fat_sectors,
            cluster_bytes: geometry.cluster_bytes,
            cluster_count: geometry.cluster_count,
            used_clusters,
            runs,
            mbr: Vec::new(),
            boot_sector: Vec::new(),
            fs_info: Vec::new(),
            label,
        };
        Ok(Self {
            mbr: image.render_mbr(options.disk_signature),
            boot_sector: image.render_boot_sector(options.volume_id),
            fs_info: image.render_fs_info(),
            ..image
        })
    }

    /// The bytes that must be written: up to the end of the last file. The
    /// free space after it is left as it is.
    pub fn image_len(&self) -> u64 {
        self.data_offset() + self.used_clusters * self.cluster_bytes
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn cluster_bytes(&self) -> u64 {
        self.cluster_bytes
    }

    pub fn partition_offset(&self) -> u64 {
        PARTITION_START_BYTES
    }

    pub fn partition_bytes(&self) -> u64 {
        self.partition_sectors * self.sector_bytes
    }

    fn fat_offset(&self) -> u64 {
        PARTITION_START_BYTES + self.reserved_sectors * self.sector_bytes
    }

    fn fat_bytes(&self) -> u64 {
        self.fat_sectors * self.sector_bytes
    }

    fn data_offset(&self) -> u64 {
        self.fat_offset() + FAT_COUNT * self.fat_bytes()
    }

    /// Generates `buffer.len()` bytes of the image from `offset`.
    pub fn read_at(
        &self,
        content: &dyn ContentSource,
        offset: u64,
        buffer: &mut [u8],
    ) -> io::Result<()> {
        let end = offset
            .checked_add(buffer.len() as u64)
            .filter(|end| *end <= self.image_len())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("read at byte {offset} runs past the end of the FAT32 image"),
                )
            })?;
        buffer.fill(0);

        let partition = PARTITION_START_BYTES;
        let sector = self.sector_bytes;
        let fixed: [(u64, &[u8]); 5] = [
            (0, &self.mbr),
            (partition, &self.boot_sector),
            (partition + FS_INFO_SECTOR * sector, &self.fs_info),
            (partition + BACKUP_BOOT_SECTOR * sector, &self.boot_sector),
            (partition + (BACKUP_BOOT_SECTOR + 1) * sector, &self.fs_info),
        ];
        for (start, bytes) in fixed {
            copy_overlap(buffer, offset, start, bytes);
        }

        for copy in 0..FAT_COUNT {
            let fat_start = self.fat_offset() + copy * self.fat_bytes();
            let fat_end = fat_start + self.fat_bytes();
            let from = offset.max(fat_start);
            let to = end.min(fat_end);
            if from < to {
                self.fill_fat(buffer, offset, fat_start, from, to);
            }
        }

        let data_start = self.data_offset();
        if end > data_start {
            self.fill_data(content, buffer, offset, offset.max(data_start), end)?;
        }
        Ok(())
    }

    /// Writes the FAT entries that fall within `from..to` of the image.
    fn fill_fat(&self, buffer: &mut [u8], offset: u64, fat_start: u64, from: u64, to: u64) {
        let first_entry = (from - fat_start) / 4;
        let last_entry = (to - fat_start).div_ceil(4);
        for entry in first_entry..last_entry {
            let value = self.fat_entry(entry).to_le_bytes();
            let entry_start = fat_start + entry * 4;
            copy_overlap(buffer, offset, entry_start, &value);
        }
    }

    fn fat_entry(&self, cluster: u64) -> u32 {
        match cluster {
            0 => 0x0FFF_FF00 | 0xF8,
            1 => END_OF_CHAIN,
            _ => match self.run_for(cluster) {
                Some(run) if cluster + 1 == run.first_cluster + run.clusters => END_OF_CHAIN,
                Some(_) => (cluster + 1) as u32,
                None => 0,
            },
        }
    }

    fn run_for(&self, cluster: u64) -> Option<&Run> {
        let index = self
            .runs
            .partition_point(|run| run.first_cluster + run.clusters <= cluster);
        self.runs
            .get(index)
            .filter(|run| run.first_cluster <= cluster)
    }

    fn fill_data(
        &self,
        content: &dyn ContentSource,
        buffer: &mut [u8],
        offset: u64,
        from: u64,
        to: u64,
    ) -> io::Result<()> {
        let data_start = self.data_offset();
        let mut position = from;
        while position < to {
            let cluster = 2 + (position - data_start) / self.cluster_bytes;
            let Some(run) = self.run_for(cluster) else {
                return Err(io::Error::other(format!(
                    "cluster {cluster} is not part of the image"
                )));
            };
            let run_start = data_start + (run.first_cluster - 2) * self.cluster_bytes;
            let run_end = run_start + run.clusters * self.cluster_bytes;
            let piece_end = to.min(run_end);
            let within = position - run_start;
            let target = &mut buffer[(position - offset) as usize..(piece_end - offset) as usize];
            match &run.data {
                RunData::Folder(bytes) => {
                    let available = (bytes.len() as u64).saturating_sub(within);
                    let length = (target.len() as u64).min(available) as usize;
                    if length > 0 {
                        target[..length]
                            .copy_from_slice(&bytes[within as usize..within as usize + length]);
                    }
                }
                RunData::File {
                    content: id,
                    size_bytes,
                } => {
                    let available = size_bytes.saturating_sub(within);
                    let length = (target.len() as u64).min(available) as usize;
                    if length > 0 {
                        content.read_content(*id, within, &mut target[..length])?;
                    }
                }
            }
            position = piece_end;
        }
        Ok(())
    }

    fn render_mbr(&self, disk_signature: u32) -> Vec<u8> {
        let mut mbr = vec![0u8; 512];
        let code = stub_code(0);
        mbr[..code.len()].copy_from_slice(&code);
        mbr[440..444].copy_from_slice(&disk_signature.to_le_bytes());

        let first_sector = PARTITION_START_BYTES / self.sector_bytes;
        let last_sector = first_sector + self.partition_sectors - 1;
        let entry = &mut mbr[446..462];
        entry[0] = 0x80;
        entry[1..4].copy_from_slice(&chs(first_sector));
        entry[4] = 0x0C;
        entry[5..8].copy_from_slice(&chs(last_sector));
        entry[8..12].copy_from_slice(&(first_sector as u32).to_le_bytes());
        entry[12..16].copy_from_slice(&(self.partition_sectors as u32).to_le_bytes());
        mbr[510] = 0x55;
        mbr[511] = 0xAA;
        mbr
    }

    fn render_boot_sector(&self, volume_id: u32) -> Vec<u8> {
        let mut boot = vec![0u8; self.sector_bytes as usize];
        boot[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
        boot[3..11].copy_from_slice(b"MSWIN4.1");
        boot[11..13].copy_from_slice(&(self.sector_bytes as u16).to_le_bytes());
        boot[13] = self.sectors_per_cluster as u8;
        boot[14..16].copy_from_slice(&(self.reserved_sectors as u16).to_le_bytes());
        boot[16] = FAT_COUNT as u8;
        boot[21] = 0xF8;
        boot[24..26].copy_from_slice(&63u16.to_le_bytes());
        boot[26..28].copy_from_slice(&255u16.to_le_bytes());
        let hidden_sectors = (PARTITION_START_BYTES / self.sector_bytes) as u32;
        boot[28..32].copy_from_slice(&hidden_sectors.to_le_bytes());
        boot[32..36].copy_from_slice(&(self.partition_sectors as u32).to_le_bytes());
        boot[36..40].copy_from_slice(&(self.fat_sectors as u32).to_le_bytes());
        boot[44..48].copy_from_slice(&2u32.to_le_bytes());
        boot[48..50].copy_from_slice(&(FS_INFO_SECTOR as u16).to_le_bytes());
        boot[50..52].copy_from_slice(&(BACKUP_BOOT_SECTOR as u16).to_le_bytes());
        boot[64] = 0x80;
        boot[66] = 0x29;
        boot[67..71].copy_from_slice(&volume_id.to_le_bytes());
        boot[71..82].copy_from_slice(&label_bytes(&self.label));
        boot[82..90].copy_from_slice(b"FAT32   ");
        let code = stub_code(0x5A);
        boot[0x5A..0x5A + code.len()].copy_from_slice(&code);
        boot[510] = 0x55;
        boot[511] = 0xAA;
        boot
    }

    fn render_fs_info(&self) -> Vec<u8> {
        let mut info = vec![0u8; self.sector_bytes as usize];
        info[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
        info[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
        let free = (self.cluster_count - self.used_clusters) as u32;
        info[488..492].copy_from_slice(&free.to_le_bytes());
        info[492..496].copy_from_slice(&((2 + self.used_clusters) as u32).to_le_bytes());
        info[508..512].copy_from_slice(&0xAA55_0000u32.to_le_bytes());
        info
    }
}

/// Copies the part of `bytes` (which sits at image offset `start`) that
/// overlaps the buffer (which holds the image from `offset`).
fn copy_overlap(buffer: &mut [u8], offset: u64, start: u64, bytes: &[u8]) {
    let end = start + bytes.len() as u64;
    let buffer_end = offset + buffer.len() as u64;
    let from = offset.max(start);
    let to = buffer_end.min(end);
    if from < to {
        buffer[(from - offset) as usize..(to - offset) as usize]
            .copy_from_slice(&bytes[(from - start) as usize..(to - start) as usize]);
    }
}

struct Geometry {
    sectors_per_cluster: u64,
    cluster_bytes: u64,
    reserved_sectors: u64,
    fat_sectors: u64,
    cluster_count: u64,
}

/// Microsoft's default cluster size for the volume, made smaller when a
/// small volume would otherwise have too few clusters to be FAT32.
fn choose_geometry(total_sectors: u64, sector_bytes: u64) -> Option<Geometry> {
    let volume_bytes = total_sectors * sector_bytes;
    let mut cluster_bytes = match volume_bytes {
        bytes if bytes <= 8 * 1024 * MIB => 4096,
        bytes if bytes <= 16 * 1024 * MIB => 8192,
        bytes if bytes <= 32 * 1024 * MIB => 16 * 1024,
        _ => MAX_CLUSTER_BYTES,
    }
    .max(sector_bytes);

    loop {
        if let Some(geometry) = layout_for(total_sectors, sector_bytes, cluster_bytes) {
            return Some(geometry);
        }
        if cluster_bytes <= sector_bytes {
            return None;
        }
        cluster_bytes /= 2;
    }
}

fn layout_for(total_sectors: u64, sector_bytes: u64, cluster_bytes: u64) -> Option<Geometry> {
    let sectors_per_cluster = cluster_bytes / sector_bytes;
    let entries_per_sector = sector_bytes / 4;
    let most_clusters = total_sectors.checked_sub(RESERVED_SECTORS)? / sectors_per_cluster;
    let fat_sectors = (most_clusters + 2).div_ceil(entries_per_sector);
    // Grow the reserved area so the data region starts on a 1 MiB boundary.
    let alignment = MIB / sector_bytes;
    let data_start = RESERVED_SECTORS + FAT_COUNT * fat_sectors;
    let reserved_sectors =
        RESERVED_SECTORS + (data_start.div_ceil(alignment) * alignment - data_start);
    if reserved_sectors > u64::from(u16::MAX) {
        return None;
    }
    let metadata = reserved_sectors + FAT_COUNT * fat_sectors;
    let cluster_count = total_sectors.checked_sub(metadata)? / sectors_per_cluster;
    (MIN_CLUSTERS..MAX_CLUSTERS)
        .contains(&cluster_count)
        .then_some(Geometry {
            sectors_per_cluster,
            cluster_bytes,
            reserved_sectors,
            fat_sectors,
            cluster_count,
        })
}

/// A folder, flattened: its children in order, and where it sits in the
/// list of folders (root first, then breadth first).
struct FolderPlan {
    parent: Option<usize>,
    clusters: u64,
    children: Vec<ChildPlan>,
    files: Vec<FilePlan>,
}

struct FilePlan {
    size_bytes: u64,
    content: usize,
}

struct ChildPlan {
    kind: ChildKind,
    hidden: bool,
    short_name: [u8; 11],
    case_flags: u8,
    long_entries: Vec<[u8; 32]>,
}

#[derive(Clone, Copy)]
enum ChildKind {
    Folder(usize),
    File(u64),
}

fn plan_folders(root: &FatDirectory, cluster_bytes: u64) -> Result<Vec<FolderPlan>, FatError> {
    let mut plans: Vec<FolderPlan> = Vec::new();
    let mut queue: Vec<(&FatDirectory, Option<usize>, String)> = vec![(root, None, String::new())];
    let mut next = 0;

    while next < queue.len() {
        let (folder, parent, path) = queue[next].clone();
        let index = next;
        next += 1;

        let names = short_names(folder, &path)?;
        let mut children = Vec::with_capacity(folder.entries.len());
        let mut files = Vec::new();
        for (entry, (short_name, case_flags, long_entries)) in folder.entries.iter().zip(names) {
            match entry {
                FatEntry::Directory(child) => {
                    let child_index = queue.len();
                    queue.push((child, Some(index), format!("{path}/{}", child.name)));
                    children.push(ChildPlan {
                        kind: ChildKind::Folder(child_index),
                        hidden: child.hidden,
                        short_name,
                        case_flags,
                        long_entries,
                    });
                }
                FatEntry::File(file) => {
                    if file.size_bytes > MAX_FAT32_FILE_BYTES {
                        return Err(FatError::FileTooLarge {
                            path: format!("{path}/{}", file.name),
                            size_bytes: file.size_bytes,
                        });
                    }
                    files.push(FilePlan {
                        size_bytes: file.size_bytes,
                        content: file.content,
                    });
                    children.push(ChildPlan {
                        kind: ChildKind::File(file.size_bytes),
                        hidden: file.hidden,
                        short_name,
                        case_flags,
                        long_entries,
                    });
                }
            }
        }

        let own_entries = if parent.is_none() { 1 } else { 2 };
        let entry_count = own_entries
            + children
                .iter()
                .map(|child| 1 + child.long_entries.len() as u64)
                .sum::<u64>();
        if entry_count > MAX_FOLDER_ENTRIES {
            return Err(FatError::TooManyEntries {
                path: display_path(&path),
            });
        }
        let clusters = (entry_count * 32).div_ceil(cluster_bytes).max(1);
        plans.push(FolderPlan {
            parent,
            clusters,
            children,
            files,
        });
        debug_assert_eq!(plans.len() - 1, index);
    }

    Ok(plans)
}

fn display_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    }
}

type NameEntries = ([u8; 11], u8, Vec<[u8; 32]>);

/// Gives every entry of a folder its short name, case flags and long name
/// entries, checking names as it goes.
fn short_names(folder: &FatDirectory, path: &str) -> Result<Vec<NameEntries>, FatError> {
    let mut seen_long = HashSet::new();
    let mut used_short: HashSet<[u8; 11]> = HashSet::new();
    let mut results: Vec<Option<NameEntries>> = vec![None; folder.entries.len()];

    // Plain 8.3 names keep their own short name, so they are placed first.
    for (index, entry) in folder.entries.iter().enumerate() {
        let name = entry_name(entry);
        let full_path = format!("{path}/{name}");
        check_long_name(name, &full_path)?;
        if !seen_long.insert(name.to_uppercase()) {
            return Err(FatError::DuplicateName { path: full_path });
        }
        if let Some((short, flags)) = plain_short_name(name) {
            if !used_short.insert(short) {
                return Err(FatError::DuplicateName { path: full_path });
            }
            results[index] = Some((short, flags, Vec::new()));
        }
    }

    for (index, entry) in folder.entries.iter().enumerate() {
        if results[index].is_some() {
            continue;
        }
        let name = entry_name(entry);
        let (base, extension) = basis_name(name);
        let short = (1u32..)
            .map(|number| numbered_short_name(&base, &extension, number))
            .find(|candidate| !used_short.contains(candidate))
            .expect("a free short name");
        used_short.insert(short);
        results[index] = Some((short, 0, long_name_entries(name, &short)));
    }

    Ok(results
        .into_iter()
        .map(|result| result.expect("named"))
        .collect())
}

fn entry_name(entry: &FatEntry) -> &str {
    match entry {
        FatEntry::Directory(folder) => &folder.name,
        FatEntry::File(file) => &file.name,
    }
}

fn check_long_name(name: &str, path: &str) -> Result<(), FatError> {
    let invalid = |reason| {
        Err(FatError::InvalidName {
            path: path.to_string(),
            reason,
        })
    };
    if name.is_empty() || name == "." || name == ".." {
        return invalid("it is empty or a dot name");
    }
    if name.chars().any(|character| {
        (character as u32) < 0x20
            || matches!(
                character,
                '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|'
            )
    }) {
        return invalid("it contains a character FAT32 does not allow");
    }
    if name.ends_with(['.', ' ']) {
        return invalid("it ends with a dot or a space");
    }
    if name.encode_utf16().count() > MAX_LONG_NAME_UNITS {
        return invalid("it is longer than 255 characters");
    }
    Ok(())
}

fn is_short_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '('
                | ')'
                | '-'
                | '@'
                | '^'
                | '_'
                | '`'
                | '{'
                | '}'
                | '~'
        )
}

/// The short name and lowercase flags for a name that is already a valid
/// 8.3 name in one letter case per part, which needs no long name entries.
fn plain_short_name(name: &str) -> Option<([u8; 11], u8)> {
    let (base, extension) = match name.rfind('.') {
        Some(dot) => (&name[..dot], &name[dot + 1..]),
        None => (name, ""),
    };
    if base.is_empty()
        || base.len() > 8
        || extension.len() > 3
        || (name.contains('.') && extension.is_empty())
        || !base
            .chars()
            .chain(extension.chars())
            .all(is_short_character)
    {
        return None;
    }

    let mut flags = 0;
    for (part, flag) in [(base, LOWERCASE_BASE), (extension, LOWERCASE_EXTENSION)] {
        let has_lower = part.chars().any(|character| character.is_ascii_lowercase());
        let has_upper = part.chars().any(|character| character.is_ascii_uppercase());
        match (has_lower, has_upper) {
            (true, true) => return None,
            (true, false) => flags |= flag,
            _ => {}
        }
    }

    let mut short = [b' '; 11];
    for (index, byte) in base.bytes().enumerate() {
        short[index] = byte.to_ascii_uppercase();
    }
    for (index, byte) in extension.bytes().enumerate() {
        short[8 + index] = byte.to_ascii_uppercase();
    }
    Some((short, flags))
}

/// The characters a generated short name starts from: uppercase, spaces and
/// extra dots removed, anything not allowed in a short name made `_`.
fn basis_name(name: &str) -> (Vec<u8>, Vec<u8>) {
    let trimmed = name.trim_start_matches('.');
    let (base, extension) = match trimmed.rfind('.') {
        Some(dot) => (&trimmed[..dot], &trimmed[dot + 1..]),
        None => (trimmed, ""),
    };
    let convert = |part: &str, limit: usize| -> Vec<u8> {
        part.chars()
            .filter(|character| *character != ' ' && *character != '.')
            .map(|character| {
                let upper = character.to_ascii_uppercase();
                if is_short_character(upper) {
                    upper as u8
                } else {
                    b'_'
                }
            })
            .take(limit)
            .collect()
    };
    let mut base = convert(base, 8);
    if base.is_empty() {
        base.push(b'_');
    }
    (base, convert(extension, 3))
}

fn numbered_short_name(base: &[u8], extension: &[u8], number: u32) -> [u8; 11] {
    let tail = format!("~{number}");
    let keep = (8 - tail.len()).min(base.len());
    let mut short = [b' '; 11];
    short[..keep].copy_from_slice(&base[..keep]);
    short[keep..keep + tail.len()].copy_from_slice(tail.as_bytes());
    short[8..8 + extension.len()].copy_from_slice(extension);
    short
}

fn short_name_checksum(short: &[u8; 11]) -> u8 {
    short.iter().fold(0u8, |sum, &byte| {
        (sum >> 1).wrapping_add(sum << 7).wrapping_add(byte)
    })
}

/// Long name entries in the order they are stored: the last piece first.
fn long_name_entries(name: &str, short: &[u8; 11]) -> Vec<[u8; 32]> {
    let mut units: Vec<u16> = name.encode_utf16().collect();
    let pieces = units.len().div_ceil(13);
    if !units.len().is_multiple_of(13) {
        units.push(0x0000);
        units.resize(pieces * 13, 0xFFFF);
    }
    let checksum = short_name_checksum(short);
    let offsets = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

    (1..=pieces)
        .rev()
        .map(|sequence| {
            let mut entry = [0u8; 32];
            entry[0] = sequence as u8 | if sequence == pieces { 0x40 } else { 0 };
            entry[11] = ATTRIBUTE_LONG_NAME;
            entry[13] = checksum;
            for (position, offset) in offsets.iter().enumerate() {
                let unit = units[(sequence - 1) * 13 + position];
                entry[*offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
            }
            entry
        })
        .collect()
}

fn short_entry(
    name: &[u8; 11],
    attributes: u8,
    case_flags: u8,
    first_cluster: u64,
    size: u32,
    date: u16,
    time: u16,
) -> [u8; 32] {
    let mut entry = [0u8; 32];
    entry[..11].copy_from_slice(name);
    entry[11] = attributes;
    entry[12] = case_flags;
    entry[14..16].copy_from_slice(&time.to_le_bytes());
    entry[16..18].copy_from_slice(&date.to_le_bytes());
    entry[18..20].copy_from_slice(&date.to_le_bytes());
    entry[20..22].copy_from_slice(&((first_cluster >> 16) as u16).to_le_bytes());
    entry[22..24].copy_from_slice(&time.to_le_bytes());
    entry[24..26].copy_from_slice(&date.to_le_bytes());
    entry[26..28].copy_from_slice(&(first_cluster as u16).to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    entry
}

fn hidden_attribute(hidden: bool) -> u8 {
    if hidden {
        ATTRIBUTE_HIDDEN
    } else {
        0
    }
}

/// A FAT volume label: up to 11 uppercase short-name characters or spaces.
fn volume_label(requested: &str) -> String {
    let label: String = requested
        .chars()
        .map(|character| {
            let upper = character.to_ascii_uppercase();
            if is_short_character(upper) || upper == ' ' {
                upper
            } else {
                '_'
            }
        })
        .take(11)
        .collect();
    let label = label.trim_end().to_string();
    if label.is_empty() {
        "OSFLUX".to_string()
    } else {
        label
    }
}

fn label_bytes(label: &str) -> [u8; 11] {
    let mut bytes = [b' '; 11];
    for (index, byte) in label.bytes().take(11).enumerate() {
        bytes[index] = byte;
    }
    bytes
}

/// Cylinder, head and sector for an LBA, with the usual 255 heads and 63
/// sectors per track; past the limit, the conventional maximum value.
fn chs(lba: u64) -> [u8; 3] {
    const HEADS: u64 = 255;
    const SECTORS: u64 = 63;
    let cylinder = lba / (HEADS * SECTORS);
    if cylinder > 1023 {
        return [0xFE, 0xFF, 0xFF];
    }
    let head = (lba / SECTORS) % HEADS;
    let sector = lba % SECTORS + 1;
    [
        head as u8,
        (sector as u8) | (((cylinder >> 2) & 0xC0) as u8),
        (cylinder & 0xFF) as u8,
    ]
}

/// A few bytes of 16-bit x86 code that print [`UEFI_ONLY_MESSAGE`] through
/// the BIOS and wait. `base` is where the code sits in its sector, which is
/// loaded at 0x7C00.
fn stub_code(base: u16) -> Vec<u8> {
    let message_address = 0x7C00 + base + 32;
    let mut code = vec![
        0xFA, // cli
        0x31,
        0xC0, // xor ax, ax
        0x8E,
        0xD8, // mov ds, ax
        0x8E,
        0xD0, // mov ss, ax
        0xBC,
        0x00,
        0x7C, // mov sp, 0x7C00
        0xFB, // sti
        0xBE,
        (message_address & 0xFF) as u8,
        (message_address >> 8) as u8, // mov si, message
        0xFC,                         // cld
        0xAC,                         // print: lodsb
        0x08,
        0xC0, // or al, al
        0x74,
        0x09, // jz halt
        0xB4,
        0x0E, // mov ah, 0x0E
        0xBB,
        0x07,
        0x00, // mov bx, 0x0007
        0xCD,
        0x10, // int 0x10
        0xEB,
        0xF2, // jmp print
        0xF4, // halt: hlt
        0xEB,
        0xFD, // jmp halt
    ];
    debug_assert_eq!(code.len(), 32);
    code.extend_from_slice(UEFI_ONLY_MESSAGE);
    code
}

#[cfg(test)]
mod tests {
    use super::{
        basis_name, numbered_short_name, plain_short_name, short_name_checksum, ContentSource,
        FatDirectory, FatEntry, FatError, FatFile, FatImage, FatImageOptions, END_OF_CHAIN, MIB,
    };
    use crate::drive_contents::{inspect_drive_contents, DriveContentsState, FilesystemKind};
    use crate::test_support::SparseDrive;
    use std::collections::HashMap;
    use std::io;

    const GIB: u64 = 1024 * MIB;

    /// File contents kept in memory, by content number.
    struct Contents(HashMap<usize, Vec<u8>>);

    impl ContentSource for Contents {
        fn read_content(&self, content: usize, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
            let bytes = &self.0[&content];
            buffer.copy_from_slice(&bytes[offset as usize..offset as usize + buffer.len()]);
            Ok(())
        }
    }

    struct TreeBuilder {
        contents: HashMap<usize, Vec<u8>>,
    }

    impl TreeBuilder {
        fn file(&mut self, name: &str, bytes: Vec<u8>) -> FatEntry {
            let content = self.contents.len();
            let size_bytes = bytes.len() as u64;
            self.contents.insert(content, bytes);
            FatEntry::File(FatFile {
                name: name.to_string(),
                hidden: false,
                size_bytes,
                content,
            })
        }
    }

    fn folder(name: &str, entries: Vec<FatEntry>) -> FatEntry {
        FatEntry::Directory(FatDirectory {
            name: name.to_string(),
            hidden: false,
            entries,
        })
    }

    fn pattern(seed: u8, length: usize) -> Vec<u8> {
        (0..length)
            .map(|index| seed.wrapping_add((index % 251) as u8))
            .collect()
    }

    /// A tree laid out like a Windows installer, with the awkward cases:
    /// long names, many entries, non-ASCII names, an empty file, a hidden
    /// file and a file of exactly one cluster.
    fn windows_like() -> (FatDirectory, Contents) {
        let mut tree = TreeBuilder {
            contents: HashMap::new(),
        };
        let mut logs = Vec::new();
        for number in 1..=40 {
            logs.push(tree.file(
                &format!("Microsoft-Windows-Setup-Component-Log-Number-{number:02}.etl"),
                format!("log {number:02}\n").into_bytes(),
            ));
        }
        let mut hidden = tree.file("desktop.ini", b"[.ShellClassInfo]\r\n".to_vec());
        if let FatEntry::File(file) = &mut hidden {
            file.hidden = true;
        }
        let root = FatDirectory {
            name: String::new(),
            hidden: false,
            entries: vec![
                tree.file(
                    "autorun.inf",
                    b"[AutoRun.Amd64]\r\nopen=setup.exe\r\nicon=setup.exe,0\r\n".to_vec(),
                ),
                tree.file("bootmgr", pattern(1, 4096)),
                tree.file("setup.exe", pattern(2, 9000)),
                hidden,
                folder("boot", vec![]),
                folder(
                    "efi",
                    vec![folder(
                        "boot",
                        vec![tree.file("bootx64.efi", pattern(3, 300_000))],
                    )],
                ),
                folder(
                    "sources",
                    vec![
                        tree.file("boot.wim", pattern(4, 2 * MIB as usize + 123)),
                        tree.file("empty.txt", Vec::new()),
                        folder("en-us", vec![tree.file("setup.exe.mui", pattern(5, 777))]),
                    ],
                ),
                folder(
                    "support",
                    vec![
                        folder("logging", logs),
                        tree.file("r\u{e9}sum\u{e9}.txt", "caf\u{e9}".as_bytes().to_vec()),
                        tree.file("\u{65e5}\u{672c}\u{8a9e}.txt", b"nihongo".to_vec()),
                        tree.file("Mixed Case Name.TXT", b"mixed".to_vec()),
                    ],
                ),
            ],
        };
        (root, Contents(tree.contents))
    }

    fn options(drive_bytes: u64) -> FatImageOptions {
        FatImageOptions {
            drive_bytes,
            sector_bytes: 512,
            label: "CCCOMA_X64FRE_EN-US_DV9".to_string(),
            volume_id: 0x1234_5678,
            disk_signature: 0x0ABC_DEF0,
            unix_seconds: 1_795_022_016,
        }
    }

    fn generate(image: &FatImage, contents: &Contents) -> Vec<u8> {
        let mut bytes = vec![0u8; image.image_len() as usize];
        image.read_at(contents, 0, &mut bytes).unwrap();
        bytes
    }

    fn as_drive(bytes: &[u8], drive_bytes: u64) -> SparseDrive {
        let drive = SparseDrive::empty(drive_bytes, 0);
        drive.write(0, bytes);
        drive
    }

    #[test]
    fn step_1_reads_the_image_as_one_fat32_partition() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let drive = as_drive(&generate(&image, &contents), GIB);

        let found = inspect_drive_contents(&drive);

        assert_eq!(found.state, DriveContentsState::HasData);
        assert_eq!(found.partitions.len(), 1);
        let partition = &found.partitions[0];
        assert_eq!(partition.start_bytes, MIB);
        assert_eq!(partition.size_bytes, GIB - MIB);
        assert_eq!(partition.filesystem, Some(FilesystemKind::Fat32));
        assert_eq!(partition.label.as_deref(), Some("CCCOMA_X64F"));
        assert_eq!(image.label(), "CCCOMA_X64F");
    }

    #[test]
    fn an_empty_tree_makes_an_empty_stick() {
        let root = FatDirectory {
            name: String::new(),
            hidden: false,
            entries: Vec::new(),
        };
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let drive = as_drive(&generate(&image, &Contents(HashMap::new())), GIB);

        assert_eq!(
            inspect_drive_contents(&drive).state,
            DriveContentsState::EmptyFilesystem
        );
    }

    #[test]
    fn every_file_reads_back_byte_for_byte() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let bytes = generate(&image, &contents);
        let volume = TestVolume::open(&bytes);

        for (path, expected) in [
            (
                "autorun.inf",
                b"[AutoRun.Amd64]\r\nopen=setup.exe\r\nicon=setup.exe,0\r\n".to_vec(),
            ),
            ("setup.exe", pattern(2, 9000)),
            ("efi/boot/bootx64.efi", pattern(3, 300_000)),
            ("sources/boot.wim", pattern(4, 2 * MIB as usize + 123)),
            ("sources/empty.txt", Vec::new()),
            ("sources/en-us/setup.exe.mui", pattern(5, 777)),
            (
                "support/r\u{e9}sum\u{e9}.txt",
                "caf\u{e9}".as_bytes().to_vec(),
            ),
            ("support/\u{65e5}\u{672c}\u{8a9e}.txt", b"nihongo".to_vec()),
            ("support/Mixed Case Name.TXT", b"mixed".to_vec()),
            (
                "support/logging/Microsoft-Windows-Setup-Component-Log-Number-33.etl",
                b"log 33\n".to_vec(),
            ),
        ] {
            assert_eq!(volume.read_file(path), expected, "{path}");
        }
        assert_eq!(volume.list("support/logging").len(), 40);
        assert!(volume.entry("desktop.ini").attributes & 0x02 != 0, "hidden");
    }

    #[test]
    fn plain_names_use_short_entries_with_lowercase_flags() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let bytes = generate(&image, &contents);
        let volume = TestVolume::open(&bytes);

        let setup = volume.entry("setup.exe");
        assert_eq!(&setup.short_name, b"SETUP   EXE");
        assert_eq!(setup.case_flags, 0x18);
        assert!(!setup.had_long_name);
        let mixed = volume.entry("support/Mixed Case Name.TXT");
        assert!(mixed.had_long_name);
        assert_eq!(&mixed.short_name, b"MIXEDC~1TXT");
    }

    #[test]
    fn generates_the_same_bytes_in_any_piece_size() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let whole = generate(&image, &contents);

        let mut pieces = Vec::with_capacity(whole.len());
        let mut offset = 0u64;
        while offset < image.image_len() {
            let length = 7777.min(image.image_len() - offset) as usize;
            let mut piece = vec![0xEEu8; length];
            image.read_at(&contents, offset, &mut piece).unwrap();
            pieces.extend(piece);
            offset += length as u64;
        }

        assert_eq!(pieces, whole);
        let again = FatImage::build(&root, &options(GIB)).unwrap();
        assert_eq!(generate(&again, &contents), whole);
    }

    #[test]
    fn writes_only_up_to_the_last_file() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(64 * GIB)).unwrap();

        assert!(
            image.image_len() < 8 * MIB + 64 * MIB,
            "{}",
            image.image_len()
        );
        assert_eq!(image.cluster_bytes(), 32 * 1024);
        assert!(image
            .read_at(&contents, image.image_len(), &mut [0u8; 1])
            .is_err());
    }

    #[test]
    fn writes_a_bootable_looking_mbr_with_a_uefi_only_message() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let bytes = generate(&image, &contents);

        assert_eq!(&bytes[510..512], &[0x55, 0xAA]);
        assert_eq!(bytes[446], 0x80, "active");
        assert_eq!(bytes[446 + 4], 0x0C, "FAT32 with LBA");
        assert_eq!(
            u32::from_le_bytes(bytes[454..458].try_into().unwrap()),
            2048
        );
        assert_eq!(&bytes[440..444], &0x0ABC_DEF0u32.to_le_bytes());
        let message = String::from_utf8_lossy(&bytes[32..140]);
        assert!(message.contains("UEFI"), "{message}");
        // The region before the partition is rewritten as zeros, which
        // clears an old primary GPT.
        assert!(bytes[512..MIB as usize].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn fat_chains_end_where_each_run_ends() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let bytes = generate(&image, &contents);
        let volume = TestVolume::open(&bytes);

        assert_eq!(image.fat_entry(0), 0x0FFF_FFF8);
        assert_eq!(image.fat_entry(1), END_OF_CHAIN);
        let bootx64 = volume.entry("efi/boot/bootx64.efi");
        let clusters = 300_000u64.div_ceil(image.cluster_bytes());
        let first = u64::from(bootx64.first_cluster);
        for cluster in first..first + clusters - 1 {
            assert_eq!(u64::from(image.fat_entry(cluster)), cluster + 1);
        }
        assert_eq!(image.fat_entry(first + clusters - 1), END_OF_CHAIN);
        assert_eq!(image.fat_entry(2 + image.used_clusters), 0, "free");
    }

    #[test]
    fn builds_for_4k_sector_drives() {
        let (root, contents) = windows_like();
        let mut four_k = options(GIB);
        four_k.sector_bytes = 4096;
        let image = FatImage::build(&root, &four_k).unwrap();
        let bytes = generate(&image, &contents);

        assert_eq!(
            u16::from_le_bytes([bytes[MIB as usize + 11], bytes[MIB as usize + 12]]),
            4096
        );
        assert_eq!(u32::from_le_bytes(bytes[454..458].try_into().unwrap()), 256);
        assert_eq!(
            TestVolume::open(&bytes).read_file("setup.exe"),
            pattern(2, 9000)
        );
    }

    #[test]
    fn refuses_a_file_of_4_gib_or_more() {
        let root = FatDirectory {
            name: String::new(),
            hidden: false,
            entries: vec![folder(
                "sources",
                vec![FatEntry::File(FatFile {
                    name: "install.wim".to_string(),
                    hidden: false,
                    size_bytes: 4 * GIB,
                    content: 0,
                })],
            )],
        };

        let error = FatImage::build(&root, &options(16 * GIB)).unwrap_err();

        assert_eq!(
            error,
            FatError::FileTooLarge {
                path: "/sources/install.wim".to_string(),
                size_bytes: 4 * GIB
            }
        );
    }

    #[test]
    fn refuses_names_fat32_cannot_hold() {
        let mut tree = TreeBuilder {
            contents: HashMap::new(),
        };
        for (name, expected) in [
            ("a:b.txt", "character"),
            ("trailing.", "dot or a space"),
            ("trailing ", "dot or a space"),
        ] {
            let root = FatDirectory {
                name: String::new(),
                hidden: false,
                entries: vec![tree.file(name, b"x".to_vec())],
            };
            let error = FatImage::build(&root, &options(GIB)).unwrap_err();
            assert!(error.to_string().contains(expected), "{name}: {error}");
        }
    }

    #[test]
    fn refuses_names_that_differ_only_in_case() {
        let mut tree = TreeBuilder {
            contents: HashMap::new(),
        };
        let root = FatDirectory {
            name: String::new(),
            hidden: false,
            entries: vec![
                tree.file("Setup.exe", b"a".to_vec()),
                tree.file("SETUP.EXE", b"b".to_vec()),
            ],
        };

        assert!(matches!(
            FatImage::build(&root, &options(GIB)),
            Err(FatError::DuplicateName { .. })
        ));
    }

    #[test]
    fn refuses_files_that_do_not_fit_and_drives_too_small() {
        let root = FatDirectory {
            name: String::new(),
            hidden: false,
            entries: vec![FatEntry::File(FatFile {
                name: "big.bin".to_string(),
                hidden: false,
                size_bytes: 3 * GIB,
                content: 0,
            })],
        };

        assert!(matches!(
            FatImage::build(&root, &options(2 * GIB)),
            Err(FatError::DoesNotFit { .. })
        ));
        assert!(matches!(
            FatImage::build(&root, &options(16 * MIB)),
            Err(FatError::DriveTooSmall { .. })
        ));
    }

    #[test]
    fn short_names_follow_the_8_3_rules() {
        assert_eq!(plain_short_name("setup.exe"), Some((*b"SETUP   EXE", 0x18)));
        assert_eq!(plain_short_name("EFI"), Some((*b"EFI        ", 0)));
        assert_eq!(plain_short_name("en-us"), Some((*b"EN-US      ", 0x08)));
        assert_eq!(
            plain_short_name("BOOTX64.efi"),
            Some((*b"BOOTX64 EFI", 0x10))
        );
        assert_eq!(plain_short_name("Setup.exe"), None, "mixed case");
        assert_eq!(plain_short_name("setup.exe.mui"), None, "two dots");
        assert_eq!(plain_short_name("toolongname.txt"), None);
        assert_eq!(plain_short_name("a b.txt"), None, "space");

        let (base, extension) = basis_name("Microsoft-Windows-Setup.etl");
        assert_eq!(&numbered_short_name(&base, &extension, 1), b"MICROS~1ETL");
        assert_eq!(&numbered_short_name(&base, &extension, 12), b"MICRO~12ETL");
        let (base, extension) = basis_name("r\u{e9}sum\u{e9}.txt");
        assert_eq!(&numbered_short_name(&base, &extension, 1), b"R_SUM_~1TXT");
        let (base, _) = basis_name("...");
        assert_eq!(base, b"_");
    }

    #[test]
    fn generated_short_names_are_unique_in_a_folder() {
        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let bytes = generate(&image, &contents);
        let volume = TestVolume::open(&bytes);

        let names: Vec<[u8; 11]> = volume
            .list("support/logging")
            .into_iter()
            .map(|entry| entry.short_name)
            .collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
        assert!(names.contains(b"MICROS~1ETL"));
        assert!(names.contains(b"MICRO~40ETL"));
    }

    #[test]
    fn external_tools_accept_the_image() {
        // fsck.fat and mtools read the image as an independent check. They
        // are standard on Linux; where they are missing the test says so and
        // passes.
        let tools = ["fsck.fat", "mdir", "mcopy"];
        if let Some(missing) = tools.iter().find(|tool| !tool_exists(tool)) {
            eprintln!("skipping: {missing} is not installed");
            return;
        }

        let (root, contents) = windows_like();
        let image = FatImage::build(&root, &options(GIB)).unwrap();
        let bytes = generate(&image, &contents);
        let directory = std::env::temp_dir().join(format!(
            "osflux-fat32-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let volume_path = directory.join("volume.img");
        let volume_file = std::fs::File::create(&volume_path).unwrap();
        volume_file.set_len(image.partition_bytes()).unwrap();
        use std::io::Write as _;
        (&volume_file).write_all(&bytes[MIB as usize..]).unwrap();
        drop(volume_file);

        let fsck = std::process::Command::new("fsck.fat")
            .args(["-n", "-v"])
            .arg(&volume_path)
            .output()
            .unwrap();
        let fsck_output = String::from_utf8_lossy(&fsck.stdout).into_owned()
            + &String::from_utf8_lossy(&fsck.stderr);

        let listing = std::process::Command::new("mdir")
            .env("MTOOLS_SKIP_CHECK", "1")
            .env("LC_ALL", "C.UTF-8")
            .args(["-/", "-b", "-i"])
            .arg(&volume_path)
            .arg("::")
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&listing.stdout).into_owned();

        let mut extracted = Vec::new();
        for (path, expected) in [
            ("efi/boot/bootx64.efi", pattern(3, 300_000)),
            ("sources/boot.wim", pattern(4, 2 * MIB as usize + 123)),
            (
                "support/r\u{e9}sum\u{e9}.txt",
                "caf\u{e9}".as_bytes().to_vec(),
            ),
        ] {
            let output = std::process::Command::new("mcopy")
                .env("MTOOLS_SKIP_CHECK", "1")
                .env("LC_ALL", "C.UTF-8")
                .args(["-n", "-i"])
                .arg(&volume_path)
                .arg(format!("::/{path}"))
                .arg("-")
                .output()
                .unwrap();
            extracted.push((path, output.stdout, expected));
        }
        let _ = std::fs::remove_dir_all(&directory);

        eprintln!("{fsck_output}");
        assert!(
            fsck.status.success(),
            "fsck.fat found problems:\n{fsck_output}"
        );
        assert!(
            !fsck_output.contains("Free cluster summary wrong"),
            "{fsck_output}"
        );
        for expected in [
            "::/setup.exe",
            "::/efi/boot/bootx64.efi",
            "::/sources/en-us/setup.exe.mui",
            "::/support/logging/Microsoft-Windows-Setup-Component-Log-Number-40.etl",
            "::/support/Mixed Case Name.TXT",
        ] {
            assert!(
                listing.contains(expected),
                "{expected} missing from:\n{listing}"
            );
        }
        for (path, actual, expected) in extracted {
            assert_eq!(actual, expected, "{path} extracted by mcopy differs");
        }
    }

    fn tool_exists(tool: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|directory| directory.join(tool).is_file())
        })
    }

    #[test]
    fn checksum_matches_the_specification_example() {
        // The checksum Windows stores for "SETUP~1 EXE"-style names is a
        // rotate-and-add over the 11 bytes.
        let mut expected = 0u8;
        for &byte in b"MICROS~1ETL" {
            expected = expected.rotate_right(1).wrapping_add(byte);
        }
        assert_eq!(short_name_checksum(b"MICROS~1ETL"), expected);
    }

    // A small FAT32 reader for the tests: follows folders and files by name,
    // matching long names when present.

    struct TestVolume<'a> {
        bytes: &'a [u8],
        sector: u64,
        sectors_per_cluster: u64,
        fat_offset: u64,
        data_offset: u64,
        root_cluster: u32,
    }

    #[derive(Debug, Clone)]
    struct TestEntry {
        name: String,
        short_name: [u8; 11],
        case_flags: u8,
        attributes: u8,
        first_cluster: u32,
        size: u32,
        had_long_name: bool,
    }

    impl<'a> TestVolume<'a> {
        fn open(bytes: &'a [u8]) -> Self {
            // The partition starts at 1 MiB whatever the sector size.
            let start = MIB;
            let boot = &bytes[start as usize..];
            let sector = u64::from(u16::from_le_bytes([boot[11], boot[12]]));
            let partition_sector =
                u64::from(u32::from_le_bytes(bytes[454..458].try_into().unwrap()));
            assert_eq!(partition_sector * sector, start);
            let reserved = u64::from(u16::from_le_bytes([boot[14], boot[15]]));
            let fats = u64::from(boot[16]);
            let fat_sectors = u64::from(u32::from_le_bytes(boot[36..40].try_into().unwrap()));
            Self {
                bytes,
                sector,
                sectors_per_cluster: u64::from(boot[13]),
                fat_offset: start + reserved * sector,
                data_offset: start + (reserved + fats * fat_sectors) * sector,
                root_cluster: u32::from_le_bytes(boot[44..48].try_into().unwrap()),
            }
        }

        fn cluster_bytes(&self) -> u64 {
            self.sector * self.sectors_per_cluster
        }

        fn chain(&self, first: u32) -> Vec<u32> {
            let mut clusters = Vec::new();
            let mut cluster = first;
            while (2..0x0FFF_FFF8).contains(&cluster) {
                clusters.push(cluster);
                let at = (self.fat_offset + u64::from(cluster) * 4) as usize;
                cluster =
                    u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap()) & 0x0FFF_FFFF;
            }
            clusters
        }

        fn cluster_data(&self, cluster: u32) -> &[u8] {
            let start = (self.data_offset + u64::from(cluster - 2) * self.cluster_bytes()) as usize;
            &self.bytes[start..start + self.cluster_bytes() as usize]
        }

        fn folder_entries(&self, first: u32) -> Vec<TestEntry> {
            let data: Vec<u8> = self
                .chain(first)
                .into_iter()
                .flat_map(|cluster| self.cluster_data(cluster).to_vec())
                .collect();
            let mut entries = Vec::new();
            let mut long_units: Vec<(u8, Vec<u16>)> = Vec::new();
            for raw in data.chunks_exact(32) {
                if raw[0] == 0 {
                    break;
                }
                if raw[11] == 0x0F {
                    let units = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30]
                        .iter()
                        .map(|&offset| u16::from_le_bytes([raw[offset], raw[offset + 1]]))
                        .collect();
                    long_units.push((raw[0] & 0x1F, units));
                    continue;
                }
                if raw[11] & 0x08 != 0 || raw[0] == b'.' {
                    long_units.clear();
                    continue;
                }
                let short_name: [u8; 11] = raw[..11].try_into().unwrap();
                let had_long_name = !long_units.is_empty();
                let name = if had_long_name {
                    long_units.sort_by_key(|(sequence, _)| *sequence);
                    let units: Vec<u16> = long_units
                        .drain(..)
                        .flat_map(|(_, units)| units)
                        .take_while(|&unit| unit != 0 && unit != 0xFFFF)
                        .collect();
                    String::from_utf16(&units).unwrap()
                } else {
                    let base = String::from_utf8_lossy(&short_name[..8])
                        .trim_end()
                        .to_string();
                    let extension = String::from_utf8_lossy(&short_name[8..])
                        .trim_end()
                        .to_string();
                    let base = if raw[12] & 0x08 != 0 {
                        base.to_lowercase()
                    } else {
                        base
                    };
                    let extension = if raw[12] & 0x10 != 0 {
                        extension.to_lowercase()
                    } else {
                        extension
                    };
                    if extension.is_empty() {
                        base
                    } else {
                        format!("{base}.{extension}")
                    }
                };
                entries.push(TestEntry {
                    name,
                    short_name,
                    case_flags: raw[12],
                    attributes: raw[11],
                    first_cluster: (u32::from(u16::from_le_bytes([raw[20], raw[21]])) << 16)
                        | u32::from(u16::from_le_bytes([raw[26], raw[27]])),
                    size: u32::from_le_bytes(raw[28..32].try_into().unwrap()),
                    had_long_name,
                });
            }
            entries
        }

        fn entry(&self, path: &str) -> TestEntry {
            let mut folder = self.root_cluster;
            let parts: Vec<&str> = path.split('/').collect();
            for (index, part) in parts.iter().enumerate() {
                let entry = self
                    .folder_entries(folder)
                    .into_iter()
                    .find(|entry| entry.name == *part)
                    .unwrap_or_else(|| panic!("{part} not found in {path}"));
                if index + 1 == parts.len() {
                    return entry;
                }
                folder = entry.first_cluster;
            }
            unreachable!()
        }

        fn list(&self, path: &str) -> Vec<TestEntry> {
            self.folder_entries(self.entry(path).first_cluster)
        }

        fn read_file(&self, path: &str) -> Vec<u8> {
            let entry = self.entry(path);
            let mut data: Vec<u8> = self
                .chain(entry.first_cluster)
                .into_iter()
                .flat_map(|cluster| self.cluster_data(cluster).to_vec())
                .collect();
            data.truncate(entry.size as usize);
            data
        }
    }
}
