//! Splits a Windows image (`.wim`) into parts (`.swm`) small enough for
//! FAT32, the format `Dism /Split-Image` writes and Windows Setup reads.
//!
//! A WIM is a header, compressed resources (one per unique file, plus one
//! metadata resource per image), a table of those resources and an XML
//! description. Each part gets a copy of the header marked as part n of N,
//! its share of the resources copied unchanged, its own resource table and
//! the XML. The metadata resources go in part 1. Nothing is decompressed or
//! recompressed, and nothing is written to a temporary file: a part is
//! described as pieces of the original plus a few new bytes, and generated
//! on demand.

use std::fmt;
use std::io;

use crate::image_io::ImageRead;

/// The part size Microsoft's documentation uses with `Dism /Split-Image`.
pub const DEFAULT_PART_BYTES: u64 = 3800 * 1024 * 1024;

const HEADER_BYTES: usize = 208;
const MAGIC: &[u8; 8] = b"MSWIM\0\0\0";
const VERSION_DEFAULT: u32 = 0x0001_0D00;
const VERSION_SOLID: u32 = 0x0000_0E00;
const TABLE_ENTRY_BYTES: usize = 50;
/// The resource table is read into memory. A Windows install.wim with every
/// edition has a few hundred thousand entries, around 10 MiB; a table far
/// beyond that is refused rather than allocated.
const MAX_TABLE_BYTES: u64 = 256 * 1024 * 1024;

const HEADER_FLAG_SPANNED: u32 = 0x0000_0008;
const HEADER_FLAG_WRITE_IN_PROGRESS: u32 = 0x0000_0040;

const RESOURCE_FLAG_FREE: u8 = 0x01;
const RESOURCE_FLAG_METADATA: u8 = 0x02;
const RESOURCE_FLAG_COMPRESSED: u8 = 0x04;
const RESOURCE_FLAG_SPANNED: u8 = 0x08;
const RESOURCE_FLAG_SOLID: u8 = 0x10;

const OFFSET_FLAGS: usize = 16;
const OFFSET_PART_NUMBER: usize = 40;
const OFFSET_TOTAL_PARTS: usize = 42;
const OFFSET_IMAGE_COUNT: usize = 44;
const OFFSET_TABLE: usize = 48;
const OFFSET_XML: usize = 72;
const OFFSET_BOOT: usize = 96;
const OFFSET_BOOT_INDEX: usize = 120;
const OFFSET_INTEGRITY: usize = 124;

#[derive(Debug)]
pub enum WimSplitError {
    NotWim,
    /// A solid-compressed image, such as an ESD file, which cannot be split
    /// without recompressing it.
    Solid,
    AlreadySplit,
    Unsupported(String),
    Damaged(String),
    /// One resource is too large to fit any part.
    ResourceTooLarge {
        bytes: u64,
        limit: u64,
    },
    Io(io::Error),
}

impl fmt::Display for WimSplitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWim => formatter.write_str("the file is not a Windows image (WIM)"),
            Self::Solid => formatter.write_str(
                "the image uses solid compression, as ESD files do, and cannot be split without recompressing it",
            ),
            Self::AlreadySplit => formatter.write_str("the image is already split into parts"),
            Self::Unsupported(what) => write!(formatter, "the image uses {what}, which OSflux cannot split"),
            Self::Damaged(what) => write!(formatter, "the image is damaged: {what}"),
            Self::ResourceTooLarge { bytes, limit } => write!(
                formatter,
                "one piece of the image is {bytes} bytes, more than a part can hold ({limit} bytes)"
            ),
            Self::Io(error) => write!(formatter, "could not read the image: {error}"),
        }
    }
}

impl std::error::Error for WimSplitError {}

impl From<io::Error> for WimSplitError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn damaged<T>(what: impl Into<String>) -> Result<T, WimSplitError> {
    Err(WimSplitError::Damaged(what.into()))
}

/// A resource header: where a resource is and how large it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResourceHeader {
    size_in_wim: u64,
    flags: u8,
    offset: u64,
    original_size: u64,
}

impl ResourceHeader {
    fn parse(bytes: &[u8]) -> Self {
        let mut size = [0u8; 8];
        size[..7].copy_from_slice(&bytes[..7]);
        Self {
            size_in_wim: u64::from_le_bytes(size),
            flags: bytes[7],
            offset: u64_at(bytes, 8),
            original_size: u64_at(bytes, 16),
        }
    }

    fn encode(&self) -> [u8; 24] {
        let mut bytes = [0u8; 24];
        bytes[..7].copy_from_slice(&self.size_in_wim.to_le_bytes()[..7]);
        bytes[7] = self.flags;
        bytes[8..16].copy_from_slice(&self.offset.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.original_size.to_le_bytes());
        bytes
    }

    fn end(&self) -> Option<u64> {
        self.offset.checked_add(self.size_in_wim)
    }
}

/// One entry of the resource table.
#[derive(Debug, Clone, Copy)]
struct TableEntry {
    resource: ResourceHeader,
    reference_count: u32,
    hash: [u8; 20],
}

impl TableEntry {
    fn encode(&self, resource: ResourceHeader, part_number: u16) -> [u8; TABLE_ENTRY_BYTES] {
        let mut bytes = [0u8; TABLE_ENTRY_BYTES];
        bytes[..24].copy_from_slice(&resource.encode());
        bytes[24..26].copy_from_slice(&part_number.to_le_bytes());
        bytes[26..30].copy_from_slice(&self.reference_count.to_le_bytes());
        bytes[30..50].copy_from_slice(&self.hash);
        bytes
    }

    fn is_metadata(&self) -> bool {
        self.resource.flags & RESOURCE_FLAG_METADATA != 0
    }
}

/// A piece of a part: new bytes, or a range copied from the original image.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Bytes(Vec<u8>),
    Source { offset: u64, length: u64 },
}

impl Segment {
    fn len(&self) -> u64 {
        match self {
            Self::Bytes(bytes) => bytes.len() as u64,
            Self::Source { length, .. } => *length,
        }
    }
}

/// One `.swm` file, described as pieces to generate it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwmPart {
    pub part_number: u16,
    pub size_bytes: u64,
    segments: Vec<Segment>,
}

impl SwmPart {
    /// Generates `buffer.len()` bytes of this part from `offset`, reading
    /// copied ranges from `source`, the original image.
    pub fn read_at(
        &self,
        source: &dyn ImageRead,
        offset: u64,
        buffer: &mut [u8],
    ) -> io::Result<()> {
        let end = offset
            .checked_add(buffer.len() as u64)
            .filter(|end| *end <= self.size_bytes)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "read past the end of the part",
                )
            })?;
        let mut segment_start = 0u64;
        for segment in &self.segments {
            let segment_end = segment_start + segment.len();
            let from = offset.max(segment_start);
            let to = end.min(segment_end);
            if from < to {
                let target = &mut buffer[(from - offset) as usize..(to - offset) as usize];
                let within = from - segment_start;
                match segment {
                    Segment::Bytes(bytes) => target
                        .copy_from_slice(&bytes[within as usize..within as usize + target.len()]),
                    Segment::Source { offset, .. } => {
                        source.read_image_at(offset + within, target)?
                    }
                }
            }
            segment_start = segment_end;
            if segment_start >= end {
                break;
            }
        }
        Ok(())
    }
}

/// The parts an image splits into, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WimSplit {
    pub parts: Vec<SwmPart>,
}

/// File names for the parts of `install.wim`: `install.swm`,
/// `install2.swm`, `install3.swm` and so on, as DISM names them.
pub fn part_names(wim_name: &str, count: usize) -> Vec<String> {
    let stem = match wim_name.rfind('.') {
        Some(dot) => &wim_name[..dot],
        None => wim_name,
    };
    (1..=count)
        .map(|number| {
            if number == 1 {
                format!("{stem}.swm")
            } else {
                format!("{stem}{number}.swm")
            }
        })
        .collect()
}

/// Plans splitting the image `source` into parts of at most `part_limit`
/// bytes. A single resource larger than that may take a part of its own, up
/// to `hard_limit` bytes (FAT32's largest file).
pub fn plan_wim_split(
    source: &dyn ImageRead,
    part_limit: u64,
    hard_limit: u64,
) -> Result<WimSplit, WimSplitError> {
    let image_len = source.image_len()?;
    if image_len < HEADER_BYTES as u64 {
        return Err(WimSplitError::NotWim);
    }
    let mut header = [0u8; HEADER_BYTES];
    source.read_image_at(0, &mut header)?;
    if &header[..8] != MAGIC {
        return Err(WimSplitError::NotWim);
    }
    if u32_at(&header, 8) as usize != HEADER_BYTES {
        return damaged("the header has an unexpected size");
    }
    match u32_at(&header, 12) {
        VERSION_DEFAULT => {}
        VERSION_SOLID => return Err(WimSplitError::Solid),
        other => {
            return Err(WimSplitError::Unsupported(format!(
                "format version {other:#x}"
            )))
        }
    }
    let flags = u32_at(&header, OFFSET_FLAGS);
    if flags & HEADER_FLAG_SPANNED != 0
        || u16_at(&header, OFFSET_PART_NUMBER) != 1
        || u16_at(&header, OFFSET_TOTAL_PARTS) != 1
    {
        return Err(WimSplitError::AlreadySplit);
    }
    if flags & HEADER_FLAG_WRITE_IN_PROGRESS != 0 {
        return damaged("the image was never finished being written");
    }

    let table = ResourceHeader::parse(&header[OFFSET_TABLE..OFFSET_TABLE + 24]);
    let xml = ResourceHeader::parse(&header[OFFSET_XML..OFFSET_XML + 24]);
    let boot = ResourceHeader::parse(&header[OFFSET_BOOT..OFFSET_BOOT + 24]);
    let boot_index = u32_at(&header, OFFSET_BOOT_INDEX);
    let image_count = u32_at(&header, OFFSET_IMAGE_COUNT);

    if table.flags & RESOURCE_FLAG_COMPRESSED != 0 {
        return Err(WimSplitError::Unsupported(
            "a compressed resource table".to_string(),
        ));
    }
    for (name, resource) in [("resource table", table), ("XML data", xml)] {
        if resource.end().is_none_or(|end| end > image_len) {
            return damaged(format!("the {name} runs past the end of the image"));
        }
    }
    if !table.size_in_wim.is_multiple_of(TABLE_ENTRY_BYTES as u64) {
        return damaged("the resource table has a partial entry");
    }
    if table.size_in_wim > MAX_TABLE_BYTES {
        return Err(WimSplitError::Unsupported(format!(
            "its resource table is {} bytes, more than the {MAX_TABLE_BYTES} OSflux reads",
            table.size_in_wim
        )));
    }

    let mut table_bytes = vec![0u8; table.size_in_wim as usize];
    source.read_image_at(table.offset, &mut table_bytes)?;
    let mut entries = Vec::new();
    for raw in table_bytes.chunks_exact(TABLE_ENTRY_BYTES) {
        let resource = ResourceHeader::parse(&raw[..24]);
        if resource.flags & RESOURCE_FLAG_FREE != 0 {
            continue;
        }
        if resource.flags & RESOURCE_FLAG_SOLID != 0 {
            return Err(WimSplitError::Solid);
        }
        if resource.flags & RESOURCE_FLAG_SPANNED != 0 {
            return Err(WimSplitError::Unsupported(
                "a resource spanning parts".to_string(),
            ));
        }
        if u16_at(raw, 24) != 1 {
            return Err(WimSplitError::AlreadySplit);
        }
        if resource.end().is_none_or(|end| end > image_len) || resource.offset < HEADER_BYTES as u64
        {
            return damaged("a resource lies outside the image");
        }
        entries.push(TableEntry {
            resource,
            reference_count: u32_at(raw, 26),
            hash: raw[30..50].try_into().expect("20 bytes"),
        });
    }
    let metadata_count = entries.iter().filter(|entry| entry.is_metadata()).count();
    if metadata_count != image_count as usize {
        return damaged(format!(
            "the header lists {image_count} images but there are {metadata_count} metadata resources"
        ));
    }

    if boot_index != 0
        && !entries
            .iter()
            .any(|entry| entry.is_metadata() && entry.resource == boot)
    {
        return damaged("the boot image's metadata is not in the resource table");
    }

    // Metadata first, so it lands in part 1; then the rest in image order.
    entries.sort_by_key(|entry| (!entry.is_metadata(), entry.resource.offset));
    let overhead = HEADER_BYTES as u64 + xml.size_in_wim;
    let mut groups: Vec<Vec<TableEntry>> = vec![Vec::new()];
    let mut size = overhead;
    for entry in entries {
        let needed = entry.resource.size_in_wim + TABLE_ENTRY_BYTES as u64;
        let current = groups.last_mut().expect("a group");
        let starts_new_part = !current.is_empty()
            && size + needed > part_limit
            && !(entry.is_metadata() && current.iter().all(TableEntry::is_metadata));
        if starts_new_part {
            groups.push(Vec::new());
            size = overhead;
        }
        size += needed;
        if size > hard_limit {
            return Err(WimSplitError::ResourceTooLarge {
                bytes: entry.resource.size_in_wim,
                limit: hard_limit - overhead,
            });
        }
        groups.last_mut().expect("a group").push(entry);
    }
    if groups.len() > usize::from(u16::MAX) {
        return Err(WimSplitError::Unsupported(
            "more parts than a split image can number".to_string(),
        ));
    }

    let total_parts = groups.len() as u16;
    let parts = groups
        .into_iter()
        .enumerate()
        .map(|(index, mut group)| {
            let part_number = index as u16 + 1;
            group.sort_by_key(|entry| entry.resource.offset);
            build_part(
                &header,
                part_number,
                total_parts,
                &group,
                xml,
                boot,
                boot_index,
                table.flags,
            )
        })
        .collect();

    Ok(WimSplit { parts })
}

#[allow(clippy::too_many_arguments)]
fn build_part(
    original_header: &[u8; HEADER_BYTES],
    part_number: u16,
    total_parts: u16,
    entries: &[TableEntry],
    xml: ResourceHeader,
    boot: ResourceHeader,
    boot_index: u32,
    table_flags: u8,
) -> SwmPart {
    let mut segments = Vec::with_capacity(entries.len() + 3);
    let mut table = Vec::with_capacity(entries.len() * TABLE_ENTRY_BYTES);
    let mut position = HEADER_BYTES as u64;
    let mut new_boot = ResourceHeader {
        size_in_wim: 0,
        flags: 0,
        offset: 0,
        original_size: 0,
    };

    for entry in entries {
        let moved = ResourceHeader {
            offset: position,
            ..entry.resource
        };
        if part_number == 1 && boot_index != 0 && entry.resource == boot {
            new_boot = moved;
        }
        table.extend_from_slice(&entry.encode(moved, part_number));
        segments.push(Segment::Source {
            offset: entry.resource.offset,
            length: entry.resource.size_in_wim,
        });
        position += entry.resource.size_in_wim;
    }

    let table_header = ResourceHeader {
        size_in_wim: table.len() as u64,
        flags: table_flags,
        offset: position,
        original_size: table.len() as u64,
    };
    position += table.len() as u64;
    let xml_header = ResourceHeader {
        offset: position,
        ..xml
    };
    position += xml.size_in_wim;

    let mut header = *original_header;
    let flags =
        (u32_at(&header, OFFSET_FLAGS) | HEADER_FLAG_SPANNED) & !HEADER_FLAG_WRITE_IN_PROGRESS;
    header[OFFSET_FLAGS..OFFSET_FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
    header[OFFSET_PART_NUMBER..OFFSET_PART_NUMBER + 2].copy_from_slice(&part_number.to_le_bytes());
    header[OFFSET_TOTAL_PARTS..OFFSET_TOTAL_PARTS + 2].copy_from_slice(&total_parts.to_le_bytes());
    header[OFFSET_TABLE..OFFSET_TABLE + 24].copy_from_slice(&table_header.encode());
    header[OFFSET_XML..OFFSET_XML + 24].copy_from_slice(&xml_header.encode());
    header[OFFSET_BOOT..OFFSET_BOOT + 24].copy_from_slice(&new_boot.encode());
    let part_boot_index = if part_number == 1 { boot_index } else { 0 };
    header[OFFSET_BOOT_INDEX..OFFSET_BOOT_INDEX + 4]
        .copy_from_slice(&part_boot_index.to_le_bytes());
    // Integrity tables are not carried over; DISM leaves them out too unless
    // asked to check integrity.
    header[OFFSET_INTEGRITY..OFFSET_INTEGRITY + 24].fill(0);

    segments.insert(0, Segment::Bytes(header.to_vec()));
    segments.push(Segment::Bytes(table));
    segments.push(Segment::Source {
        offset: xml.offset,
        length: xml.size_in_wim,
    });

    SwmPart {
        part_number,
        size_bytes: position,
        segments,
    }
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
    use super::{
        part_names, plan_wim_split, ResourceHeader, WimSplit, WimSplitError, HEADER_BYTES,
        HEADER_FLAG_SPANNED, MAX_TABLE_BYTES, RESOURCE_FLAG_METADATA, TABLE_ENTRY_BYTES,
    };
    use crate::image_io::ImageRead;
    use crate::test_support::SparseDrive;
    use std::collections::HashMap;

    const MIB: u64 = 1024 * 1024;

    fn small_wim() -> SparseDrive {
        SparseDrive::from_fixture(include_str!("../tests/fixtures/wims/small-lzx.wim.sparse"))
    }

    fn whole(image: &dyn ImageRead) -> Vec<u8> {
        let mut bytes = vec![0u8; image.image_len().unwrap() as usize];
        image.read_image_at(0, &mut bytes).unwrap();
        bytes
    }

    fn generate(split: &WimSplit, source: &dyn ImageRead) -> Vec<Vec<u8>> {
        split
            .parts
            .iter()
            .map(|part| {
                let mut bytes = vec![0u8; part.size_bytes as usize];
                part.read_at(source, 0, &mut bytes).unwrap();
                bytes
            })
            .collect()
    }

    struct Parsed {
        header: Vec<u8>,
        entries: Vec<(ResourceHeader, u16, [u8; 20])>,
        xml: Vec<u8>,
    }

    fn parse(bytes: &[u8]) -> Parsed {
        let table = ResourceHeader::parse(&bytes[48..72]);
        let xml = ResourceHeader::parse(&bytes[72..96]);
        let entries = bytes[table.offset as usize..(table.offset + table.size_in_wim) as usize]
            .chunks_exact(TABLE_ENTRY_BYTES)
            .map(|raw| {
                (
                    ResourceHeader::parse(&raw[..24]),
                    u16::from_le_bytes([raw[24], raw[25]]),
                    raw[30..50].try_into().unwrap(),
                )
            })
            .collect();
        Parsed {
            header: bytes[..HEADER_BYTES].to_vec(),
            entries,
            xml: bytes[xml.offset as usize..(xml.offset + xml.size_in_wim) as usize].to_vec(),
        }
    }

    #[test]
    fn splits_into_parts_that_carry_every_resource_exactly_once() {
        let source = small_wim();
        let original_bytes = whole(&source);
        let original = parse(&original_bytes);

        let split = plan_wim_split(&source, 4096, MIB).unwrap();
        let parts = generate(&split, &source);

        assert!(parts.len() >= 3, "{} parts", parts.len());
        let mut seen: HashMap<[u8; 20], u16> = HashMap::new();
        for (index, bytes) in parts.iter().enumerate() {
            let number = index as u16 + 1;
            let part = parse(bytes);
            assert_eq!(&part.header[..8], b"MSWIM\0\0\0");
            assert_eq!(
                u16::from_le_bytes([part.header[40], part.header[41]]),
                number
            );
            assert_eq!(
                u16::from_le_bytes([part.header[42], part.header[43]]),
                parts.len() as u16
            );
            assert_eq!(&part.header[24..40], &original.header[24..40], "same GUID");
            let flags = u32::from_le_bytes(part.header[16..20].try_into().unwrap());
            let original_flags = u32::from_le_bytes(original.header[16..20].try_into().unwrap());
            assert_eq!(flags, original_flags | HEADER_FLAG_SPANNED);
            assert_eq!(part.xml, original.xml, "every part carries the XML");
            assert_eq!(bytes.len() as u64, split.parts[index].size_bytes);

            for (resource, part_number, hash) in &part.entries {
                assert_eq!(*part_number, number);
                assert!(
                    seen.insert(*hash, number).is_none(),
                    "resource listed twice"
                );
                let (source_resource, _, _) = original
                    .entries
                    .iter()
                    .find(|(_, _, original_hash)| original_hash == hash)
                    .expect("resource from the original");
                assert_eq!(resource.size_in_wim, source_resource.size_in_wim);
                assert_eq!(resource.flags, source_resource.flags);
                assert_eq!(resource.original_size, source_resource.original_size);
                let copied = &bytes
                    [resource.offset as usize..(resource.offset + resource.size_in_wim) as usize];
                let from = &original_bytes[source_resource.offset as usize
                    ..(source_resource.offset + source_resource.size_in_wim) as usize];
                assert_eq!(copied, from, "copied unchanged");
                if resource.flags & RESOURCE_FLAG_METADATA != 0 {
                    assert_eq!(number, 1, "metadata belongs in part 1");
                }
            }
        }
        assert_eq!(seen.len(), original.entries.len());
    }

    #[test]
    fn keeps_parts_within_the_limit_unless_one_resource_needs_more() {
        let source = small_wim();
        let split = plan_wim_split(&source, 4096, MIB).unwrap();
        let parts = generate(&split, &source);

        for (index, bytes) in parts.iter().enumerate().skip(1) {
            let part = parse(bytes);
            assert!(
                bytes.len() <= 4096 || part.entries.len() == 1,
                "part {} is {} bytes with {} resources",
                index + 1,
                bytes.len(),
                part.entries.len()
            );
        }
    }

    #[test]
    fn an_image_that_fits_one_part_gives_one_part() {
        let source = small_wim();

        let split = plan_wim_split(&source, MIB, MIB).unwrap();

        assert_eq!(split.parts.len(), 1);
    }

    #[test]
    fn generates_the_same_bytes_in_any_piece_size() {
        let source = small_wim();
        let split = plan_wim_split(&source, 4096, MIB).unwrap();
        let expected = generate(&split, &source);

        for (part, whole) in split.parts.iter().zip(&expected) {
            let mut pieces = Vec::new();
            let mut offset = 0;
            while offset < part.size_bytes {
                let length = 333.min(part.size_bytes - offset) as usize;
                let mut piece = vec![0u8; length];
                part.read_at(&source, offset, &mut piece).unwrap();
                pieces.extend(piece);
                offset += length as u64;
            }
            assert_eq!(&pieces, whole);
        }
    }

    #[test]
    fn refuses_solid_images_such_as_esd_files() {
        let solid = SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/wims/small-solid.wim.sparse"
        ));

        assert!(matches!(
            plan_wim_split(&solid, 4096, MIB),
            Err(WimSplitError::Solid)
        ));
    }

    #[test]
    fn refuses_what_is_not_a_wim_or_is_already_split() {
        let zeros = SparseDrive::empty(4096, 0);
        assert!(matches!(
            plan_wim_split(&zeros, 4096, MIB),
            Err(WimSplitError::NotWim)
        ));

        let split = small_wim();
        split.write(16, &(0x0004_008Au32).to_le_bytes());
        assert!(matches!(
            plan_wim_split(&split, 4096, MIB),
            Err(WimSplitError::AlreadySplit)
        ));
    }

    #[test]
    fn refuses_a_resource_outside_the_image() {
        let source = small_wim();
        let table = ResourceHeader::parse(&whole(&source)[48..72]);
        // Point the first resource far past the end of the file.
        source.write(table.offset + 8, &u64::MAX.to_le_bytes());

        let error = plan_wim_split(&source, 4096, MIB).unwrap_err();

        assert!(matches!(error, WimSplitError::Damaged(_)), "{error}");
    }

    #[test]
    fn refuses_a_resource_table_too_large_to_read() {
        // The small image inside a large sparse one, with its table header
        // claiming more entries than OSflux reads into memory.
        let bytes = whole(&small_wim());
        let source = SparseDrive::empty(512 * MIB, 0);
        source.write(0, &bytes);
        let huge = (MAX_TABLE_BYTES / 50 + 1) * 50;
        source.write(48, &huge.to_le_bytes()[..7]);

        let error = plan_wim_split(&source, 4096, MIB).unwrap_err();

        assert!(matches!(error, WimSplitError::Unsupported(_)), "{error}");
    }

    #[test]
    fn refuses_a_resource_too_large_for_any_part() {
        let source = small_wim();

        let error = plan_wim_split(&source, 1500, 2000).unwrap_err();

        assert!(
            matches!(error, WimSplitError::ResourceTooLarge { .. }),
            "{error}"
        );
    }

    #[test]
    fn names_parts_as_dism_does() {
        assert_eq!(
            part_names("install.wim", 3),
            ["install.swm", "install2.swm", "install3.swm"]
        );
        assert_eq!(part_names("INSTALL.WIM", 1), ["INSTALL.swm"]);
    }

    #[test]
    fn wimlib_applies_the_split_parts_exactly_like_the_original() {
        // wimlib-imagex reads the parts as an independent check. Where it is
        // missing the test says so and passes.
        if !crate::test_support::tool_exists("wimlib-imagex") {
            eprintln!("skipping: wimlib-imagex is not installed");
            return;
        }
        let source = small_wim();
        let split = plan_wim_split(&source, 4096, MIB).unwrap();
        let parts = generate(&split, &source);
        let directory = crate::test_support::temp_dir("wim-split");
        std::fs::write(directory.join("original.wim"), whole(&source)).unwrap();
        for (name, bytes) in part_names("test.wim", parts.len()).iter().zip(&parts) {
            std::fs::write(directory.join(name), bytes).unwrap();
        }

        let run = |args: &[&str]| {
            std::process::Command::new("wimlib-imagex")
                .args(args)
                .current_dir(&directory)
                .output()
                .unwrap()
        };
        let verify = run(&["verify", "test.swm", "--ref=test*.swm"]);
        let info = run(&["info", "test.swm", "--header"]);
        let apply_split = run(&["apply", "test.swm", "1", "from-split", "--ref=test*.swm"]);
        let apply_original = run(&["apply", "original.wim", "1", "from-original"]);
        let split_files = crate::test_support::read_tree(&directory.join("from-split"));
        let original_files = crate::test_support::read_tree(&directory.join("from-original"));
        let _ = std::fs::remove_dir_all(&directory);

        assert!(
            verify.status.success(),
            "wimlib-imagex verify failed:\n{}{}",
            String::from_utf8_lossy(&verify.stdout),
            String::from_utf8_lossy(&verify.stderr)
        );
        let info = String::from_utf8_lossy(&info.stdout);
        assert!(
            info.contains(&format!("Total Parts                 = {}", parts.len())),
            "{info}"
        );
        assert!(
            apply_split.status.success(),
            "{}",
            String::from_utf8_lossy(&apply_split.stderr)
        );
        assert!(apply_original.status.success());
        assert!(!original_files.is_empty());
        assert_eq!(split_files, original_files);
    }
}
