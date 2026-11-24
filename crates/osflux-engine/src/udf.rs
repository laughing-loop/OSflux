//! Reads the file tree of a UDF filesystem, which is where Windows installer
//! ISOs keep their files (their ISO 9660 part only holds a README).
//!
//! It supports what disc images use: UDF 1.02 to 2.01 with 2048-byte blocks,
//! physical (type 1) partition maps, file entries and extended file entries,
//! and short, long and embedded allocation descriptors, including allocation
//! extent continuation. Anything else (metadata or sparable partitions,
//! symbolic links, extended allocation descriptors) is refused with a reason
//! instead of being guessed at, and every descriptor's tag checksum, CRC and
//! recorded location are checked before it is trusted.

use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;

use crate::crc::crc16_itu;
use crate::image_io::ImageRead;

/// UDF on disc images always uses 2048-byte logical blocks.
pub const UDF_BLOCK_SIZE: u64 = 2048;

const FIRST_RECOGNITION_SECTOR: u64 = 16;
const MAX_RECOGNITION_SECTORS: u64 = 64;
const ANCHOR_BLOCK: u64 = 256;
const MAX_VOLUME_DESCRIPTOR_BLOCKS: u64 = 64;
/// Guards against damaged or hostile images.
const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 500_000;
const MAX_DIRECTORY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ALLOCATION_EXTENTS: usize = 4096;
const MAX_EXTENT_LENGTH: u32 = 0x3FFF_FFFF;

const TAG_PRIMARY_VOLUME: u16 = 1;
const TAG_ANCHOR: u16 = 2;
const TAG_VOLUME_POINTER: u16 = 3;
const TAG_IMPLEMENTATION_USE: u16 = 4;
const TAG_PARTITION: u16 = 5;
const TAG_LOGICAL_VOLUME: u16 = 6;
const TAG_UNALLOCATED_SPACE: u16 = 7;
const TAG_TERMINATING: u16 = 8;
const TAG_FILE_SET: u16 = 256;
const TAG_FILE_IDENTIFIER: u16 = 257;
const TAG_ALLOCATION_EXTENT: u16 = 258;
const TAG_FILE_ENTRY: u16 = 261;
const TAG_EXTENDED_FILE_ENTRY: u16 = 266;

const FILE_TYPE_DIRECTORY: u8 = 4;
const FILE_TYPE_FILE: u8 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfVolume {
    /// The logical volume identifier, such as `CCCOMA_X64FRE_EN-US_DV9`.
    pub label: String,
    pub root: UdfDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfDirectory {
    /// Empty for the root folder.
    pub name: String,
    pub hidden: bool,
    pub entries: Vec<UdfEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UdfEntry {
    Directory(UdfDirectory),
    File(UdfFile),
}

impl UdfEntry {
    pub fn name(&self) -> &str {
        match self {
            Self::Directory(directory) => &directory.name,
            Self::File(file) => &file.name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfFile {
    pub name: String,
    pub hidden: bool,
    pub size_bytes: u64,
    pub data: FileData,
}

/// Where a file's bytes are, in the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileData {
    /// Pieces of the image, in order, adding up to the file size.
    Extents(Vec<DataExtent>),
    /// Small files can live inside their file entry.
    Embedded(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataExtent {
    /// Where the piece starts in the image; `None` for a piece that was
    /// allocated but never recorded, which reads as zeros.
    pub image_offset: Option<u64>,
    pub length: u64,
}

impl UdfFile {
    /// Reads `buffer.len()` bytes of the file from `offset`.
    pub fn read_at(&self, image: &dyn ImageRead, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let end = offset
            .checked_add(buffer.len() as u64)
            .filter(|end| *end <= self.size_bytes)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("read past the end of {}", self.name),
                )
            })?;

        match &self.data {
            FileData::Embedded(bytes) => {
                buffer.copy_from_slice(&bytes[offset as usize..end as usize]);
            }
            FileData::Extents(extents) => {
                let mut extent_start = 0u64;
                let mut filled = 0usize;
                for extent in extents {
                    let extent_end = extent_start + extent.length;
                    let position = offset + filled as u64;
                    if filled < buffer.len() && position < extent_end {
                        let within = position - extent_start;
                        let length = ((extent_end - position) as usize).min(buffer.len() - filled);
                        let target = &mut buffer[filled..filled + length];
                        match extent.image_offset {
                            Some(start) => image.read_image_at(start + within, target)?,
                            None => target.fill(0),
                        }
                        filled += length;
                    }
                    extent_start = extent_end;
                }
            }
        }
        Ok(())
    }
}

impl UdfVolume {
    /// Every file with its path from the root, folders first, in the order
    /// the image lists them.
    pub fn files(&self) -> Vec<(String, &UdfFile)> {
        let mut files = Vec::new();
        collect_files(&self.root, "", &mut files);
        files
    }

    /// The entry at `path` (folders separated by `/`), compared without
    /// regard to letter case as Windows does.
    pub fn find(&self, path: &str) -> Option<&UdfEntry> {
        let mut directory = &self.root;
        let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();
        while let Some(part) = parts.next() {
            let entry = directory
                .entries
                .iter()
                .find(|entry| entry.name().eq_ignore_ascii_case(part))?;
            if parts.peek().is_none() {
                return Some(entry);
            }
            match entry {
                UdfEntry::Directory(child) => directory = child,
                UdfEntry::File(_) => return None,
            }
        }
        None
    }
}

fn collect_files<'a>(
    directory: &'a UdfDirectory,
    prefix: &str,
    files: &mut Vec<(String, &'a UdfFile)>,
) {
    for entry in &directory.entries {
        match entry {
            UdfEntry::File(file) => files.push((format!("{prefix}{}", file.name), file)),
            UdfEntry::Directory(child) => {
                collect_files(child, &format!("{prefix}{}/", child.name), files)
            }
        }
    }
}

#[derive(Debug)]
pub enum UdfError {
    /// The image has no UDF filesystem.
    NotUdf,
    /// The UDF filesystem uses a feature OSflux does not read.
    Unsupported(String),
    /// A descriptor failed its checks or points somewhere impossible.
    Damaged(String),
    Io(io::Error),
}

impl fmt::Display for UdfError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUdf => formatter.write_str("the image has no UDF filesystem"),
            Self::Unsupported(what) => write!(
                formatter,
                "the UDF filesystem uses {what}, which OSflux does not read"
            ),
            Self::Damaged(what) => write!(formatter, "the UDF filesystem is damaged: {what}"),
            Self::Io(error) => write!(formatter, "could not read the image: {error}"),
        }
    }
}

impl std::error::Error for UdfError {}

impl From<io::Error> for UdfError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn damaged<T>(what: impl Into<String>) -> Result<T, UdfError> {
    Err(UdfError::Damaged(what.into()))
}

/// Reads the UDF file tree of the image at `path`.
pub fn read_udf_file(path: impl AsRef<Path>) -> Result<UdfVolume, UdfError> {
    let file = File::open(path)?;
    read_udf(&file)
}

/// Reads the UDF file tree of `image`.
pub fn read_udf(image: &dyn ImageRead) -> Result<UdfVolume, UdfError> {
    let reader = Reader::open(image)?;
    reader.read_volume()
}

/// A partition as the logical volume refers to it.
#[derive(Debug, Clone, Copy)]
struct Partition {
    start_block: u64,
    length_blocks: u64,
}

/// Where a descriptor lives: a logical block in one of the partitions.
#[derive(Debug, Clone, Copy)]
struct LongAd {
    block: u32,
    partition: u16,
}

impl LongAd {
    fn parse(bytes: &[u8]) -> Self {
        Self {
            block: u32_at(bytes, 4),
            partition: u16_at(bytes, 8),
        }
    }
}

struct FileEntry {
    file_type: u8,
    size_bytes: u64,
    data: FileData,
}

struct Reader<'a> {
    image: &'a dyn ImageRead,
    image_blocks: u64,
    label: String,
    /// Indexed by partition reference number.
    partitions: Vec<Partition>,
    file_set: LongAd,
}

impl<'a> Reader<'a> {
    fn open(image: &'a dyn ImageRead) -> Result<Self, UdfError> {
        let image_blocks = image.image_len()? / UDF_BLOCK_SIZE;
        if image_blocks <= ANCHOR_BLOCK {
            return Err(UdfError::NotUdf);
        }
        if !has_udf_recognition_sequence(image, image_blocks)? {
            return Err(UdfError::NotUdf);
        }

        let mut reader = Self {
            image,
            image_blocks,
            label: String::new(),
            partitions: Vec::new(),
            file_set: LongAd {
                block: 0,
                partition: 0,
            },
        };

        let anchor = reader.read_anchor()?;
        let main = (u32_at(&anchor, 16), u32_at(&anchor, 20));
        let reserve = (u32_at(&anchor, 24), u32_at(&anchor, 28));
        // The reserve copy only stands in when the main sequence is unusable.
        if let Err(main_error) = reader.read_volume_descriptors(main) {
            reader
                .read_volume_descriptors(reserve)
                .map_err(|_| main_error)?;
        }
        Ok(reader)
    }

    fn read_block(&self, block: u64) -> Result<Vec<u8>, UdfError> {
        if block >= self.image_blocks {
            return damaged(format!("block {block} is past the end of the image"));
        }
        let mut buffer = vec![0u8; UDF_BLOCK_SIZE as usize];
        self.image
            .read_image_at(block * UDF_BLOCK_SIZE, &mut buffer)?;
        Ok(buffer)
    }

    fn read_anchor(&self) -> Result<Vec<u8>, UdfError> {
        let mut candidates = vec![ANCHOR_BLOCK, self.image_blocks - 1];
        if self.image_blocks > 2 * ANCHOR_BLOCK {
            candidates.push(self.image_blocks - 1 - ANCHOR_BLOCK);
        }
        for block in candidates {
            let data = self.read_block(block)?;
            if check_tag(&data, TAG_ANCHOR, block as u32).is_ok() {
                return Ok(data);
            }
        }
        damaged("no valid anchor volume descriptor pointer")
    }

    fn read_volume_descriptors(&mut self, (length, location): (u32, u32)) -> Result<(), UdfError> {
        let blocks = u64::from(length)
            .div_ceil(UDF_BLOCK_SIZE)
            .min(MAX_VOLUME_DESCRIPTOR_BLOCKS);
        let mut partition_descriptors: Vec<(u16, Partition)> = Vec::new();
        let mut logical_volume: Option<Vec<u8>> = None;
        let mut primary_label = String::new();

        for index in 0..blocks {
            let block = u64::from(location) + index;
            let data = self.read_block(block)?;
            let tag_id = u16_at(&data, 0);
            check_tag(&data, tag_id, block as u32)?;
            match tag_id {
                TAG_PRIMARY_VOLUME => primary_label = decode_dstring(&data[24..56]),
                TAG_PARTITION => {
                    let number = u16_at(&data, 22);
                    let partition = Partition {
                        start_block: u64::from(u32_at(&data, 188)),
                        length_blocks: u64::from(u32_at(&data, 192)),
                    };
                    partition_descriptors.retain(|(existing, _)| *existing != number);
                    partition_descriptors.push((number, partition));
                }
                TAG_LOGICAL_VOLUME => logical_volume = Some(data),
                TAG_IMPLEMENTATION_USE | TAG_UNALLOCATED_SPACE => {}
                TAG_TERMINATING => break,
                TAG_VOLUME_POINTER => {
                    return Err(UdfError::Unsupported(
                        "a volume descriptor pointer".to_string(),
                    ))
                }
                other => {
                    return damaged(format!(
                        "unexpected descriptor {other} in the volume descriptor sequence"
                    ))
                }
            }
        }

        let Some(logical_volume) = logical_volume else {
            return damaged("no logical volume descriptor");
        };
        let block_size = u32_at(&logical_volume, 212);
        if u64::from(block_size) != UDF_BLOCK_SIZE {
            return Err(UdfError::Unsupported(format!(
                "a logical block size of {block_size} bytes"
            )));
        }

        let map_table_length = u32_at(&logical_volume, 264) as usize;
        let map_count = u32_at(&logical_volume, 268) as usize;
        if 440 + map_table_length > logical_volume.len() {
            return damaged("the partition map table runs past its descriptor");
        }
        let maps = &logical_volume[440..440 + map_table_length];
        let mut partitions = Vec::with_capacity(map_count);
        let mut offset = 0;
        for _ in 0..map_count {
            if offset + 2 > maps.len() {
                return damaged("a partition map runs past the map table");
            }
            let (map_type, map_length) = (maps[offset], usize::from(maps[offset + 1]));
            if map_type != 1 {
                return Err(UdfError::Unsupported(format!(
                    "partition map type {map_type} (virtual, sparable or metadata partitions)"
                )));
            }
            if map_length != 6 || offset + 6 > maps.len() {
                return damaged("a type 1 partition map has the wrong length");
            }
            let number = u16_at(maps, offset + 4);
            let Some((_, partition)) = partition_descriptors
                .iter()
                .find(|(existing, _)| *existing == number)
            else {
                return damaged(format!("partition {number} has no partition descriptor"));
            };
            if partition.start_block + partition.length_blocks > self.image_blocks {
                return damaged(format!("partition {number} runs past the end of the image"));
            }
            partitions.push(*partition);
            offset += map_length;
        }
        if partitions.is_empty() {
            return damaged("the logical volume has no partitions");
        }

        let label = decode_dstring(&logical_volume[84..212]);
        self.label = if label.is_empty() {
            primary_label
        } else {
            label
        };
        self.partitions = partitions;
        self.file_set = LongAd::parse(&logical_volume[248..264]);
        Ok(())
    }

    /// The image block holding logical `block` of partition `partition`.
    fn image_block(&self, partition: u16, block: u32, blocks: u64) -> Result<u64, UdfError> {
        let Some(partition_info) = self.partitions.get(usize::from(partition)) else {
            return damaged(format!(
                "a reference to partition {partition}, which does not exist"
            ));
        };
        if u64::from(block) + blocks > partition_info.length_blocks {
            return damaged(format!(
                "block {block} (+{blocks}) is outside its partition"
            ));
        }
        Ok(partition_info.start_block + u64::from(block))
    }

    fn read_logical_block(&self, partition: u16, block: u32) -> Result<Vec<u8>, UdfError> {
        let image_block = self.image_block(partition, block, 1)?;
        self.read_block(image_block)
    }

    fn read_volume(&self) -> Result<UdfVolume, UdfError> {
        let file_set = self.read_logical_block(self.file_set.partition, self.file_set.block)?;
        check_tag(&file_set, TAG_FILE_SET, self.file_set.block)?;
        let root_icb = LongAd::parse(&file_set[400..416]);

        let mut visited = HashSet::new();
        let mut entry_count = 0;
        let root_entry = self.read_file_entry(root_icb)?;
        if root_entry.file_type != FILE_TYPE_DIRECTORY {
            return damaged("the root is not a folder");
        }
        visited.insert((root_icb.partition, root_icb.block));
        let root = self.read_directory(
            String::new(),
            false,
            &root_entry,
            0,
            &mut visited,
            &mut entry_count,
        )?;

        Ok(UdfVolume {
            label: self.label.clone(),
            root,
        })
    }

    fn read_file_entry(&self, icb: LongAd) -> Result<FileEntry, UdfError> {
        let data = self.read_logical_block(icb.partition, icb.block)?;
        let tag_id = u16_at(&data, 0);
        if tag_id != TAG_FILE_ENTRY && tag_id != TAG_EXTENDED_FILE_ENTRY {
            return damaged(format!(
                "block {} holds descriptor {tag_id}, not a file entry",
                icb.block
            ));
        }
        check_tag(&data, tag_id, icb.block)?;

        let strategy = u16_at(&data, 20);
        if strategy != 4 {
            return Err(UdfError::Unsupported(format!("ICB strategy {strategy}")));
        }
        let file_type = data[27];
        let flags = u16_at(&data, 34);
        let size_bytes = u64_at(&data, 56);
        let (ea_length, ad_length, header) = if tag_id == TAG_FILE_ENTRY {
            (
                u32_at(&data, 168) as usize,
                u32_at(&data, 172) as usize,
                176,
            )
        } else {
            (
                u32_at(&data, 208) as usize,
                u32_at(&data, 212) as usize,
                216,
            )
        };
        let ad_start = header + ea_length;
        if ad_start
            .checked_add(ad_length)
            .is_none_or(|end| end > data.len())
        {
            return damaged(format!(
                "the file entry at block {} runs past its block",
                icb.block
            ));
        }
        let descriptors = &data[ad_start..ad_start + ad_length];

        let data = match flags & 0x7 {
            3 => {
                if size_bytes > ad_length as u64 {
                    return damaged("an embedded file is larger than its file entry");
                }
                FileData::Embedded(descriptors[..size_bytes as usize].to_vec())
            }
            kind @ (0 | 1) => {
                let extents = self.collect_extents(kind, icb.partition, descriptors)?;
                FileData::Extents(trim_extents(extents, size_bytes)?)
            }
            2 => {
                return Err(UdfError::Unsupported(
                    "extended allocation descriptors".to_string(),
                ))
            }
            other => return damaged(format!("allocation descriptor type {other}")),
        };

        Ok(FileEntry {
            file_type,
            size_bytes,
            data,
        })
    }

    /// Follows a file's allocation descriptors, including continuation
    /// blocks, and turns them into image extents.
    fn collect_extents(
        &self,
        kind: u16,
        partition: u16,
        first: &[u8],
    ) -> Result<Vec<DataExtent>, UdfError> {
        let descriptor_size = if kind == 0 { 8 } else { 16 };
        let mut extents = Vec::new();
        let mut descriptors = first.to_vec();
        let mut continuations = 0;

        'list: loop {
            for raw in descriptors.chunks_exact(descriptor_size) {
                let length_field = u32_at(raw, 0);
                let length = length_field & MAX_EXTENT_LENGTH;
                let extent_type = length_field >> 30;
                if length == 0 {
                    break 'list;
                }
                let (block, extent_partition) = if kind == 0 {
                    (u32_at(raw, 4), partition)
                } else {
                    (u32_at(raw, 4), u16_at(raw, 8))
                };

                if extent_type == 3 {
                    continuations += 1;
                    if continuations > MAX_ALLOCATION_EXTENTS {
                        return damaged("too many allocation extent blocks");
                    }
                    let next = self.read_logical_block(extent_partition, block)?;
                    check_tag(&next, TAG_ALLOCATION_EXTENT, block)?;
                    let next_length = u32_at(&next, 20) as usize;
                    if 24 + next_length > next.len() {
                        return damaged("an allocation extent runs past its block");
                    }
                    descriptors = next[24..24 + next_length].to_vec();
                    continue 'list;
                }

                if extents.len() >= MAX_ALLOCATION_EXTENTS {
                    return damaged("a file has too many extents");
                }
                let blocks = u64::from(length).div_ceil(UDF_BLOCK_SIZE);
                let image_offset = if extent_type == 0 {
                    Some(self.image_block(extent_partition, block, blocks)? * UDF_BLOCK_SIZE)
                } else {
                    None
                };
                extents.push(DataExtent {
                    image_offset,
                    length: u64::from(length),
                });
            }
            break;
        }

        Ok(extents)
    }

    fn read_directory(
        &self,
        name: String,
        hidden: bool,
        entry: &FileEntry,
        depth: usize,
        visited: &mut HashSet<(u16, u32)>,
        entry_count: &mut usize,
    ) -> Result<UdfDirectory, UdfError> {
        if depth > MAX_DEPTH {
            return damaged("folders are nested too deeply");
        }
        if entry.size_bytes > MAX_DIRECTORY_BYTES {
            return damaged("a folder is too large");
        }
        let file = UdfFile {
            name: name.clone(),
            hidden,
            size_bytes: entry.size_bytes,
            data: entry.data.clone(),
        };
        let mut listing = vec![0u8; entry.size_bytes as usize];
        file.read_at(self.image, 0, &mut listing)?;

        let mut directory = UdfDirectory {
            name,
            hidden,
            entries: Vec::new(),
        };
        let mut offset = 0;
        while offset < listing.len() {
            if listing.len() - offset < 38 {
                return damaged("a file identifier is cut short");
            }
            let identifier = &listing[offset..];
            if u16_at(identifier, 0) != TAG_FILE_IDENTIFIER {
                return damaged("a folder holds something other than file identifiers");
            }
            let characteristics = identifier[18];
            let name_length = usize::from(identifier[19]);
            let icb = LongAd::parse(&identifier[20..36]);
            let use_length = usize::from(u16_at(identifier, 36));
            let record_length = (38 + use_length + name_length).div_ceil(4) * 4;
            if offset + 38 + use_length + name_length > listing.len() {
                return damaged("a file identifier runs past its folder");
            }
            // Some writers count the padding in the CRC and some do not, so
            // the CRC may cover anything up to the padded record.
            check_tag_checksum(&identifier[..record_length.min(identifier.len())])?;
            let raw_name = &identifier[38 + use_length..38 + use_length + name_length];
            offset += record_length;

            // Parent links and deleted entries are not files.
            if characteristics & 0x08 != 0 || characteristics & 0x04 != 0 {
                continue;
            }

            *entry_count += 1;
            if *entry_count > MAX_ENTRIES {
                return damaged("the image lists too many files");
            }
            let child_name = decode_name(raw_name)?;
            let child_hidden = characteristics & 0x01 != 0;
            let child = self.read_file_entry(icb)?;
            match child.file_type {
                FILE_TYPE_DIRECTORY => {
                    // Files may share an entry (disc tools store duplicates
                    // once), but a folder listed twice would loop forever.
                    if !visited.insert((icb.partition, icb.block)) {
                        return damaged(format!(
                            "the folder \"{child_name}\" is listed more than once"
                        ));
                    }
                    let subdirectory = self.read_directory(
                        child_name,
                        child_hidden,
                        &child,
                        depth + 1,
                        visited,
                        entry_count,
                    )?;
                    directory.entries.push(UdfEntry::Directory(subdirectory));
                }
                FILE_TYPE_FILE => directory.entries.push(UdfEntry::File(UdfFile {
                    name: child_name,
                    hidden: child_hidden,
                    size_bytes: child.size_bytes,
                    data: child.data,
                })),
                other => {
                    return Err(UdfError::Unsupported(format!(
                        "an entry of file type {other} (\"{child_name}\"), such as a link"
                    )))
                }
            }
        }

        Ok(directory)
    }
}

/// Cuts a file's extents down to its size; the last block is usually only
/// partly used.
fn trim_extents(extents: Vec<DataExtent>, size_bytes: u64) -> Result<Vec<DataExtent>, UdfError> {
    let mut remaining = size_bytes;
    let mut trimmed = Vec::with_capacity(extents.len());
    for extent in extents {
        if remaining == 0 {
            break;
        }
        let length = extent.length.min(remaining);
        trimmed.push(DataExtent { length, ..extent });
        remaining -= length;
    }
    if remaining != 0 {
        return damaged("a file is larger than the space recorded for it");
    }
    Ok(trimmed)
}

/// A UDF descriptor starts with the recognition sequence's `NSR02` or
/// `NSR03` in the volume recognition area at sector 16.
fn has_udf_recognition_sequence(
    image: &dyn ImageRead,
    image_blocks: u64,
) -> Result<bool, UdfError> {
    let mut buffer = vec![0u8; UDF_BLOCK_SIZE as usize];
    let last = (FIRST_RECOGNITION_SECTOR + MAX_RECOGNITION_SECTORS).min(image_blocks);
    for sector in FIRST_RECOGNITION_SECTOR..last {
        image.read_image_at(sector * UDF_BLOCK_SIZE, &mut buffer)?;
        match &buffer[1..6] {
            b"NSR02" | b"NSR03" => return Ok(true),
            b"TEA01" => return Ok(false),
            b"CD001" | b"BEA01" | b"BOOT2" | b"CDW02" => {}
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// Checks a descriptor tag: its checksum, identifier, recorded location and
/// the CRC over the descriptor body.
fn check_tag(descriptor: &[u8], expected_id: u16, expected_location: u32) -> Result<(), UdfError> {
    let tag_id = u16_at(descriptor, 0);
    if tag_id != expected_id {
        return damaged(format!(
            "expected descriptor {expected_id} at block {expected_location}, found {tag_id}"
        ));
    }
    if u32_at(descriptor, 12) != expected_location {
        return damaged(format!(
            "descriptor {tag_id} at block {expected_location} says it belongs at block {}",
            u32_at(descriptor, 12)
        ));
    }
    check_tag_checksum(descriptor)
}

/// Checks the tag checksum and, when present, the CRC of a descriptor.
fn check_tag_checksum(descriptor: &[u8]) -> Result<(), UdfError> {
    if descriptor.len() < 16 {
        return damaged("a descriptor is shorter than its tag");
    }
    let checksum = descriptor[..16]
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 4)
        .fold(0u8, |sum, (_, &byte)| sum.wrapping_add(byte));
    if checksum != descriptor[4] {
        return damaged(format!(
            "descriptor {} has a bad tag checksum",
            u16_at(descriptor, 0)
        ));
    }
    let crc_length = usize::from(u16_at(descriptor, 10));
    if crc_length > 0 {
        if 16 + crc_length > descriptor.len() {
            return damaged("a descriptor's CRC covers more than the descriptor");
        }
        if crc16_itu(&descriptor[16..16 + crc_length]) != u16_at(descriptor, 8) {
            return damaged(format!(
                "descriptor {} has a bad CRC",
                u16_at(descriptor, 0)
            ));
        }
    }
    Ok(())
}

/// Decodes a file name in OSTA compressed Unicode, refusing names that could
/// not be a single path component.
fn decode_name(bytes: &[u8]) -> Result<String, UdfError> {
    let name = decode_osta(bytes)
        .ok_or_else(|| UdfError::Damaged("a file name is not valid text".to_string()))?;
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return damaged(format!("the file name \"{name}\" is not allowed"));
    }
    Ok(name)
}

/// OSTA compressed Unicode: a first byte of 8 means one byte per character
/// (Latin-1), 16 means UTF-16 big-endian.
fn decode_osta(bytes: &[u8]) -> Option<String> {
    let (&compression, rest) = bytes.split_first()?;
    match compression {
        8 => Some(rest.iter().map(|&byte| char::from(byte)).collect()),
        16 if rest.len() % 2 == 0 => {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).ok()
        }
        _ => None,
    }
}

/// A fixed-size UDF string whose last byte holds the used length.
fn decode_dstring(field: &[u8]) -> String {
    let Some((&used, _)) = field.split_last() else {
        return String::new();
    };
    let used = usize::from(used).min(field.len() - 1);
    if used == 0 {
        return String::new();
    }
    decode_osta(&field[..used]).unwrap_or_default()
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
mod tests {
    use super::{read_udf, FileData, UdfEntry, UdfError, UdfFile, UdfVolume, UDF_BLOCK_SIZE};
    use crate::crc::crc16_itu;
    use crate::image_io::ImageRead;
    use crate::test_support::SparseDrive;

    const AUTORUN: &str = "[AutoRun.Amd64]\r\nopen=setup.exe\r\nicon=setup.exe,0\r\n";

    fn windows_like() -> SparseDrive {
        SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/isos/windows-like-udf.sparse"
        ))
    }

    fn file<'a>(volume: &'a UdfVolume, path: &str) -> &'a UdfFile {
        match volume.find(path) {
            Some(UdfEntry::File(file)) => file,
            other => panic!("{path} is not a file: {other:?}"),
        }
    }

    fn contents(image: &dyn ImageRead, file: &UdfFile) -> Vec<u8> {
        let mut buffer = vec![0u8; file.size_bytes as usize];
        file.read_at(image, 0, &mut buffer).unwrap();
        buffer
    }

    // --- An image written by genisoimage ---------------------------------

    #[test]
    fn reads_the_label_and_every_file_of_a_windows_like_iso() {
        let volume = read_udf(&windows_like()).unwrap();

        assert_eq!(volume.label, "CCCOMA_X64FRE_EN-US_DV9");
        let mut paths: Vec<_> = volume.files().into_iter().map(|(path, _)| path).collect();
        paths.sort();
        let mut expected = vec![
            "autorun.inf".to_string(),
            "bootmgr".to_string(),
            "efi/boot/bootx64.efi".to_string(),
            "setup.exe".to_string(),
            "sources/boot.wim".to_string(),
            "sources/empty.txt".to_string(),
            "support/r\u{e9}sum\u{e9}.txt".to_string(),
            "support/\u{65e5}\u{672c}\u{8a9e}.txt".to_string(),
        ];
        expected.extend((1..=40).map(|number| {
            format!("support/logging/Microsoft-Windows-Setup-Component-Log-Number-{number:02}.etl")
        }));
        expected.sort();
        assert_eq!(paths, expected);
        assert!(matches!(
            volume.find("boot"),
            Some(UdfEntry::Directory(directory)) if directory.entries.is_empty()
        ));
    }

    #[test]
    fn reads_file_contents_byte_for_byte() {
        let image = windows_like();
        let volume = read_udf(&image).unwrap();

        let setup: Vec<u8> = (0..9000)
            .map(|index| ((index * 7 + 3) % 256) as u8)
            .collect();
        let loader: Vec<u8> = (0..3000).map(|index| (index % 251) as u8).collect();
        assert_eq!(contents(&image, file(&volume, "setup.exe")), setup);
        assert_eq!(
            contents(&image, file(&volume, "efi/boot/bootx64.efi")),
            loader
        );
        assert_eq!(
            contents(&image, file(&volume, "autorun.inf")),
            AUTORUN.as_bytes()
        );
        assert_eq!(
            contents(&image, file(&volume, "support/r\u{e9}sum\u{e9}.txt")),
            "caf\u{e9}".as_bytes()
        );
        assert_eq!(
            contents(
                &image,
                file(&volume, "support/\u{65e5}\u{672c}\u{8a9e}.txt")
            ),
            b"nihongo"
        );
        let log = file(
            &volume,
            "support/logging/Microsoft-Windows-Setup-Component-Log-Number-17.etl",
        );
        assert_eq!(contents(&image, log), b"log 17\n");
        assert_eq!(file(&volume, "sources/empty.txt").size_bytes, 0);
    }

    #[test]
    fn reads_across_block_boundaries_and_large_zero_files() {
        let image = windows_like();
        let volume = read_udf(&image).unwrap();

        let setup = file(&volume, "setup.exe");
        let mut piece = vec![0u8; 100];
        setup.read_at(&image, 2000, &mut piece).unwrap();
        let expected: Vec<u8> = (2000..2100)
            .map(|index| ((index * 7 + 3) % 256) as u8)
            .collect();
        assert_eq!(piece, expected);

        let wim = file(&volume, "sources/boot.wim");
        assert_eq!(wim.size_bytes, 3 * 1024 * 1024);
        let mut middle = vec![0xAAu8; 8192];
        wim.read_at(&image, 1024 * 1024, &mut middle).unwrap();
        assert!(middle.iter().all(|&byte| byte == 0));
        assert!(setup.read_at(&image, 8990, &mut [0u8; 20]).is_err());
    }

    #[test]
    fn finds_entries_without_regard_to_letter_case() {
        let volume = read_udf(&windows_like()).unwrap();

        assert!(matches!(
            volume.find("SOURCES/BOOT.WIM"),
            Some(UdfEntry::File(_))
        ));
        assert!(volume.find("sources/missing.wim").is_none());
        assert!(volume.find("setup.exe/inside").is_none());
    }

    #[test]
    fn a_plain_iso9660_image_has_no_udf() {
        let plain =
            SparseDrive::from_fixture(include_str!("../tests/fixtures/isos/plain-iso9660.sparse"));

        assert!(matches!(read_udf(&plain), Err(UdfError::NotUdf)));
    }

    #[test]
    fn a_damaged_file_entry_is_refused() {
        let image = windows_like();
        let root_block = root_entry_block(&image);
        // Change a byte inside the root folder's file entry, under its CRC.
        let byte = image.read(root_block * UDF_BLOCK_SIZE + 100, 1)[0];
        image.write(root_block * UDF_BLOCK_SIZE + 100, &[byte ^ 0xFF]);

        let error = read_udf(&image).unwrap_err();

        assert!(matches!(error, UdfError::Damaged(_)), "{error}");
        assert!(error.to_string().contains("CRC"), "{error}");
    }

    #[test]
    fn a_metadata_partition_map_is_refused_as_unsupported() {
        let image = windows_like();
        for block in logical_volume_blocks(&image) {
            let offset = block * UDF_BLOCK_SIZE;
            image.write(offset + 440, &[2]);
            reseal_tag(&image, block);
        }

        let error = read_udf(&image).unwrap_err();

        assert!(matches!(error, UdfError::Unsupported(_)), "{error}");
    }

    /// The image block of the root folder's file entry.
    fn root_entry_block(image: &SparseDrive) -> u64 {
        let anchor = image.read(256 * UDF_BLOCK_SIZE, 2048);
        let main_location = u64::from(super::u32_at(&anchor, 20));
        let (mut partition_start, mut file_set_block) = (0, 0);
        for index in 0..16 {
            let block = image.read((main_location + index) * UDF_BLOCK_SIZE, 2048);
            match super::u16_at(&block, 0) {
                5 => partition_start = u64::from(super::u32_at(&block, 188)),
                6 => file_set_block = u64::from(super::u32_at(&block, 252)),
                8 => break,
                _ => {}
            }
        }
        let file_set = image.read((partition_start + file_set_block) * UDF_BLOCK_SIZE, 2048);
        partition_start + u64::from(super::u32_at(&file_set, 404))
    }

    fn logical_volume_blocks(image: &SparseDrive) -> Vec<u64> {
        let anchor = image.read(256 * UDF_BLOCK_SIZE, 2048);
        let mut blocks = Vec::new();
        for sequence in [20, 28] {
            let location = u64::from(super::u32_at(&anchor, sequence));
            for index in 0..16 {
                let block = image.read((location + index) * UDF_BLOCK_SIZE, 2048);
                if super::u16_at(&block, 0) == 6 {
                    blocks.push(location + index);
                }
            }
        }
        assert!(!blocks.is_empty());
        blocks
    }

    /// Recomputes a descriptor's CRC and tag checksum after a test edits it.
    fn reseal_tag(image: &SparseDrive, block: u64) {
        let offset = block * UDF_BLOCK_SIZE;
        let mut data = image.read(offset, 2048);
        let crc_length = usize::from(super::u16_at(&data, 10));
        let crc = crc16_itu(&data[16..16 + crc_length]);
        data[8..10].copy_from_slice(&crc.to_le_bytes());
        seal_checksum(&mut data);
        image.write(offset, &data[..16]);
    }

    fn seal_checksum(descriptor: &mut [u8]) {
        descriptor[4] = 0;
        descriptor[4] = descriptor[..16]
            .iter()
            .fold(0u8, |sum, &byte| sum.wrapping_add(byte));
    }

    // --- A small image built here, for what genisoimage never writes ------

    /// Builds a minimal UDF image block by block. The partition starts at
    /// image block `PARTITION_START`; logical block numbers below are inside
    /// it.
    struct Builder {
        image: Vec<u8>,
    }

    const PARTITION_START: u64 = 270;
    const PARTITION_BLOCKS: u32 = 30;

    impl Builder {
        fn new() -> Self {
            let mut builder = Self {
                image: vec![
                    0u8;
                    ((PARTITION_START + u64::from(PARTITION_BLOCKS)) * UDF_BLOCK_SIZE)
                        as usize
                ],
            };
            for (sector, identifier) in [(16, b"BEA01"), (17, b"NSR02"), (18, b"TEA01")] {
                let offset = sector * UDF_BLOCK_SIZE as usize;
                builder.image[offset + 1..offset + 6].copy_from_slice(identifier);
                builder.image[offset + 6] = 1;
            }

            let mut anchor = vec![0u8; 512];
            for extent in [16, 24] {
                anchor[extent..extent + 4].copy_from_slice(&(3 * 2048u32).to_le_bytes());
                anchor[extent + 4..extent + 8].copy_from_slice(&260u32.to_le_bytes());
            }
            builder.descriptor(256, 2, 256, anchor);

            let mut partition = vec![0u8; 512];
            partition[22..24].copy_from_slice(&0u16.to_le_bytes());
            partition[188..192].copy_from_slice(&(PARTITION_START as u32).to_le_bytes());
            partition[192..196].copy_from_slice(&PARTITION_BLOCKS.to_le_bytes());
            builder.descriptor(260, 5, 260, partition);

            let mut volume = vec![0u8; 446];
            let label = b"\x08SYNTHETIC";
            volume[84..84 + label.len()].copy_from_slice(label);
            volume[211] = label.len() as u8;
            volume[212..216].copy_from_slice(&2048u32.to_le_bytes());
            volume[248..252].copy_from_slice(&2048u32.to_le_bytes());
            volume[252..256].copy_from_slice(&0u32.to_le_bytes());
            volume[264..268].copy_from_slice(&6u32.to_le_bytes());
            volume[268..272].copy_from_slice(&1u32.to_le_bytes());
            volume[440..446].copy_from_slice(&[1, 6, 1, 0, 0, 0]);
            builder.descriptor(261, 6, 261, volume);
            builder.descriptor(262, 8, 262, vec![0u8; 16]);

            let mut file_set = vec![0u8; 512];
            file_set[400..404].copy_from_slice(&2048u32.to_le_bytes());
            file_set[404..408].copy_from_slice(&1u32.to_le_bytes());
            builder.logical(0, 256, file_set);
            builder
        }

        /// Writes `body` (tag space included) at image block `block` as
        /// descriptor `id`, recording `location`, with CRC and checksum.
        fn descriptor(&mut self, block: u64, id: u16, location: u32, mut body: Vec<u8>) {
            body[0..2].copy_from_slice(&id.to_le_bytes());
            body[2..4].copy_from_slice(&2u16.to_le_bytes());
            body[12..16].copy_from_slice(&location.to_le_bytes());
            let crc_length = (body.len() - 16) as u16;
            body[10..12].copy_from_slice(&crc_length.to_le_bytes());
            let crc = crc16_itu(&body[16..]);
            body[8..10].copy_from_slice(&crc.to_le_bytes());
            seal_checksum(&mut body);
            let offset = (block * UDF_BLOCK_SIZE) as usize;
            self.image[offset..offset + body.len()].copy_from_slice(&body);
        }

        fn logical(&mut self, block: u32, id: u16, body: Vec<u8>) {
            self.descriptor(PARTITION_START + u64::from(block), id, block, body);
        }

        fn data(&mut self, block: u32, bytes: &[u8]) {
            let offset = ((PARTITION_START + u64::from(block)) * UDF_BLOCK_SIZE) as usize;
            self.image[offset..offset + bytes.len()].copy_from_slice(bytes);
        }

        /// A file entry (tag 261) or, with `extended`, an extended file
        /// entry (tag 266).
        fn file_entry(
            &mut self,
            block: u32,
            extended: bool,
            file_type: u8,
            ad_kind: u16,
            size: u64,
            descriptors: &[u8],
        ) {
            let header = if extended { 216 } else { 176 };
            let mut body = vec![0u8; header + descriptors.len()];
            body[20..22].copy_from_slice(&4u16.to_le_bytes());
            body[24..26].copy_from_slice(&1u16.to_le_bytes());
            body[27] = file_type;
            body[34..36].copy_from_slice(&ad_kind.to_le_bytes());
            body[56..64].copy_from_slice(&size.to_le_bytes());
            let (ea, ad) = if extended { (208, 212) } else { (168, 172) };
            body[ea..ea + 4].copy_from_slice(&0u32.to_le_bytes());
            body[ad..ad + 4].copy_from_slice(&(descriptors.len() as u32).to_le_bytes());
            body[header..].copy_from_slice(descriptors);
            self.logical(block, if extended { 266 } else { 261 }, body);
        }

        fn image(self) -> Vec<u8> {
            self.image
        }
    }

    /// A file identifier pointing at logical block `icb`.
    fn identifier(parent_block: u32, name: &str, icb: u32, characteristics: u8) -> Vec<u8> {
        let mut encoded = Vec::new();
        if !name.is_empty() {
            encoded.push(8);
            encoded.extend(name.bytes());
        }
        let length = 38 + encoded.len();
        let mut body = vec![0u8; length.div_ceil(4) * 4];
        body[0..2].copy_from_slice(&257u16.to_le_bytes());
        body[2..4].copy_from_slice(&2u16.to_le_bytes());
        body[12..16].copy_from_slice(&parent_block.to_le_bytes());
        body[16..18].copy_from_slice(&1u16.to_le_bytes());
        body[18] = characteristics;
        body[19] = encoded.len() as u8;
        body[20..24].copy_from_slice(&2048u32.to_le_bytes());
        body[24..28].copy_from_slice(&icb.to_le_bytes());
        body[38..38 + encoded.len()].copy_from_slice(&encoded);
        body[10..12].copy_from_slice(&((length - 16) as u16).to_le_bytes());
        let crc = crc16_itu(&body[16..length]);
        body[8..10].copy_from_slice(&crc.to_le_bytes());
        seal_checksum(&mut body);
        body
    }

    fn long_ad(length: u32, extent_type: u32, block: u32) -> Vec<u8> {
        let mut ad = vec![0u8; 16];
        ad[0..4].copy_from_slice(&(length | (extent_type << 30)).to_le_bytes());
        ad[4..8].copy_from_slice(&block.to_le_bytes());
        ad
    }

    fn short_ad(length: u32, extent_type: u32, block: u32) -> Vec<u8> {
        let mut ad = vec![0u8; 8];
        ad[0..4].copy_from_slice(&(length | (extent_type << 30)).to_le_bytes());
        ad[4..8].copy_from_slice(&block.to_le_bytes());
        ad
    }

    fn pattern(seed: u8, length: usize) -> Vec<u8> {
        (0..length)
            .map(|index| seed.wrapping_add(index as u8))
            .collect()
    }

    /// Root folder (embedded in its entry) with three files: one through long
    /// allocation descriptors and a continuation block, one embedded in an
    /// extended file entry, one with an unrecorded extent.
    fn synthetic(extra: impl FnOnce(&mut Builder, &mut Vec<u8>)) -> Vec<u8> {
        let mut builder = Builder::new();
        let mut listing = identifier(1, "", 1, 0x0A);
        listing.extend(identifier(1, "long.bin", 2, 0));
        listing.extend(identifier(1, "tiny.txt", 3, 0));
        listing.extend(identifier(1, "sparse.bin", 4, 0));
        extra(&mut builder, &mut listing);
        builder.file_entry(1, false, 4, 3, listing.len() as u64, &listing);

        let mut long = long_ad(2048, 0, 10);
        long.extend(long_ad(2048, 3, 5));
        builder.file_entry(2, false, 5, 1, 3048, &long);
        let mut continuation = vec![0u8; 24];
        let mut continued = long_ad(1000, 0, 11);
        continued.extend(long_ad(0, 0, 0));
        continuation[20..24].copy_from_slice(&(continued.len() as u32).to_le_bytes());
        continuation.extend(continued);
        builder.logical(5, 258, continuation);
        builder.data(10, &pattern(10, 2048));
        builder.data(11, &pattern(11, 2048));

        builder.file_entry(3, true, 5, 3, 9, b"hello udf");

        let mut sparse = short_ad(4096, 1, 12);
        sparse.extend(short_ad(2048, 0, 14));
        builder.file_entry(4, false, 5, 0, 6144, &sparse);
        builder.data(14, &pattern(14, 2048));
        builder.image()
    }

    #[test]
    fn reads_long_descriptors_through_a_continuation_block() {
        let image = synthetic(|_, _| {});
        let volume = read_udf(&image).unwrap();

        assert_eq!(volume.label, "SYNTHETIC");
        let long = file(&volume, "long.bin");
        let mut expected = pattern(10, 2048);
        expected.extend(&pattern(11, 2048)[..1000]);
        assert_eq!(contents(&image, long), expected);
        assert!(matches!(&long.data, FileData::Extents(extents) if extents.len() == 2));
    }

    #[test]
    fn reads_a_file_embedded_in_an_extended_file_entry() {
        let image = synthetic(|_, _| {});
        let volume = read_udf(&image).unwrap();

        let tiny = file(&volume, "tiny.txt");
        assert_eq!(tiny.data, FileData::Embedded(b"hello udf".to_vec()));
        assert_eq!(contents(&image, tiny), b"hello udf");
    }

    #[test]
    fn an_unrecorded_extent_reads_as_zeros() {
        let image = synthetic(|_, _| {});
        let volume = read_udf(&image).unwrap();

        let data = contents(&image, file(&volume, "sparse.bin"));
        assert!(data[..4096].iter().all(|&byte| byte == 0));
        assert_eq!(&data[4096..], &pattern(14, 2048)[..]);
    }

    #[test]
    fn deleted_entries_are_skipped_and_hidden_ones_kept() {
        let image = synthetic(|_, listing| {
            listing.extend(identifier(1, "gone.txt", 3, 0x04));
            listing.extend(identifier(1, "secret.txt", 3, 0x01));
        });
        let volume = read_udf(&image).unwrap();

        assert!(volume.find("gone.txt").is_none());
        assert!(file(&volume, "secret.txt").hidden);
        // secret.txt shares its file entry with tiny.txt, as disc tools do
        // for duplicate files.
        assert_eq!(contents(&image, file(&volume, "secret.txt")), b"hello udf");
    }

    #[test]
    fn a_folder_that_contains_itself_is_refused() {
        let image = synthetic(|_, listing| {
            listing.extend(identifier(1, "loop", 1, 0x02));
        });

        let error = read_udf(&image).unwrap_err();

        assert!(
            error.to_string().contains("listed more than once"),
            "{error}"
        );
    }

    #[test]
    fn a_link_is_refused_as_unsupported() {
        let image = synthetic(|builder, listing| {
            listing.extend(identifier(1, "link", 6, 0));
            builder.file_entry(6, false, 12, 3, 0, b"");
        });

        let error = read_udf(&image).unwrap_err();

        assert!(matches!(error, UdfError::Unsupported(_)), "{error}");
    }

    #[test]
    fn a_descriptor_recorded_for_another_block_is_refused() {
        let mut builder = Builder::new();
        builder.file_entry(1, false, 4, 3, 0, b"");
        // A file entry that says it belongs at block 9, stored at block 1.
        let offset = ((PARTITION_START + 1) * UDF_BLOCK_SIZE) as usize;
        let mut image = builder.image();
        image[offset + 12..offset + 16].copy_from_slice(&9u32.to_le_bytes());
        let mut tag = image[offset..offset + 16].to_vec();
        seal_checksum(&mut tag);
        image[offset..offset + 16].copy_from_slice(&tag);

        let error = read_udf(&image).unwrap_err();

        assert!(error.to_string().contains("belongs at block 9"), "{error}");
    }

    #[test]
    fn a_file_pointing_outside_its_partition_is_refused() {
        let image = synthetic(|builder, listing| {
            listing.extend(identifier(1, "far.bin", 7, 0));
            builder.file_entry(7, false, 5, 0, 2048, &short_ad(2048, 0, 1000));
        });

        let error = read_udf(&image).unwrap_err();

        assert!(
            error.to_string().contains("outside its partition"),
            "{error}"
        );
    }

    #[test]
    fn rejects_names_that_are_not_a_single_path_component() {
        for name in ["..", "a/b"] {
            let image = synthetic(|_, listing| {
                listing.extend(identifier(1, name, 3, 0));
            });

            let error = read_udf(&image).unwrap_err();

            assert!(error.to_string().contains("not allowed"), "{name}: {error}");
        }
    }
}
