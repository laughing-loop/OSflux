//! Copies the files of a UDF ISO, such as a Windows installer, onto a new
//! MBR drive with one FAT32 partition that starts in UEFI mode.
//!
//! FAT32 cannot hold a file of 4 GiB or more. An `install.wim` that large is
//! split into `install.swm`, `install2.swm` and so on, which Windows Setup
//! reads in its place; the parts are generated from the ISO as the stick is
//! written. Any other file that large is refused.
//!
//! The layout is planned before the user confirms, so they see what will be
//! written and anything that cannot work is refused first. At write time it
//! is built again from the same ISO and the same recorded values, and must
//! come out the same size, so the written bytes are exactly the planned ones.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;

use crate::fat32::{
    ContentSource, FatDirectory, FatEntry, FatError, FatFile, FatImage, FatImageOptions,
    MAX_FAT32_FILE_BYTES,
};
use crate::fingerprint::stable_fingerprint;
use crate::image_io::ImageRead;
use crate::udf::{read_udf, UdfDirectory, UdfEntry, UdfError, UdfFile, UdfVolume};
use crate::wim_split::{part_names, plan_wim_split, SwmPart, WimSplitError, DEFAULT_PART_BYTES};

/// The one large file Windows Setup accepts in split form.
const SPLITTABLE_NAME: &str = "install.wim";

/// A file written as several parts because FAT32 cannot hold it whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitFile {
    pub path: String,
    pub size_bytes: u64,
    /// The part names, in order, such as `install.swm`, `install2.swm`.
    pub parts: Vec<String>,
}

/// What copying the ISO's files will put on the drive, recorded in the write
/// plan so the same image can be built again at write time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UefiFat32Layout {
    /// The FAT32 volume label, from the ISO's label cut to 11 characters.
    pub label: String,
    /// Files written to the stick, counting each part of a split file.
    pub file_count: usize,
    pub file_bytes: u64,
    /// The ISO's largest file and its size, for the summary shown before
    /// writing.
    pub largest_file: Option<(String, u64)>,
    pub split_files: Vec<SplitFile>,
    /// Bytes of the drive image written, from the MBR to the end of the last
    /// file. The free space after it is not written.
    pub image_bytes: u64,
    pub sector_bytes: u32,
    pub cluster_bytes: u64,
    pub volume_id: u32,
    pub disk_signature: u32,
    pub created_unix_seconds: u64,
}

/// What an ISO's UDF filesystem holds, as far as copying it onto FAT32 is
/// concerned. Shown by `--plan-image`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoFilesSummary {
    pub label: String,
    pub file_count: usize,
    pub file_bytes: u64,
    pub largest_file: Option<(String, u64)>,
    /// The path of `efi/boot/boot*.efi`, when there is one.
    pub uefi_loader: Option<String>,
    /// Large files that would be written in parts.
    pub split_files: Vec<SplitFile>,
    /// Why files could not be copied, one line each.
    pub cannot_copy: Vec<String>,
}

#[derive(Debug)]
pub enum FileCopyError {
    /// The image has no UDF filesystem to copy files from.
    NoUdf,
    Udf(UdfError),
    /// There is no `efi/boot/boot*.efi`, so the copy could not start a PC.
    NoUefiLoader,
    FileTooLarge {
        path: String,
        size_bytes: u64,
    },
    CannotSplit {
        path: String,
        reason: WimSplitError,
    },
    Fat(FatError),
    Image(io::Error),
}

impl fmt::Display for FileCopyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoUdf => formatter.write_str(
                "this ISO has no UDF filesystem; OSflux copies the files of UDF ISOs such as Windows installers",
            ),
            Self::Udf(error) => write!(formatter, "{error}"),
            Self::NoUefiLoader => formatter.write_str(
                "this ISO has no UEFI boot loader (efi/boot/boot*.efi), so a stick made from it could not start a PC",
            ),
            Self::FileTooLarge { path, size_bytes } => write!(
                formatter,
                "{path} is {}, and FAT32 cannot hold a file of 4 GiB or more; only install.wim can be split into parts",
                gib(*size_bytes)
            ),
            Self::CannotSplit { path, reason } => write!(
                formatter,
                "{path} is too large for FAT32 and cannot be split: {reason}"
            ),
            Self::Fat(error) => write!(formatter, "{error}"),
            Self::Image(error) => write!(formatter, "could not read the ISO: {error}"),
        }
    }
}

impl std::error::Error for FileCopyError {}

impl From<UdfError> for FileCopyError {
    fn from(error: UdfError) -> Self {
        match error {
            UdfError::NotUdf => Self::NoUdf,
            UdfError::Io(error) => Self::Image(error),
            other => Self::Udf(other),
        }
    }
}

fn gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// When a file is too large and how large the parts of a split file are.
/// Tests use small values to exercise splitting with small images.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CopyLimits {
    pub(crate) max_file_bytes: u64,
    pub(crate) part_bytes: u64,
}

impl Default for CopyLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_FAT32_FILE_BYTES,
            part_bytes: DEFAULT_PART_BYTES,
        }
    }
}

/// Where each file on the stick gets its bytes.
enum FileSource {
    Udf(UdfFile),
    /// A part of a split WIM; `wim` indexes [`CopyTree::wims`].
    SplitPart {
        wim: usize,
        part: SwmPart,
    },
}

/// The FAT32 tree for an ISO, with where every file's bytes come from.
struct CopyTree {
    root: FatDirectory,
    sources: Vec<FileSource>,
    wims: Vec<UdfFile>,
    split_files: Vec<SplitFile>,
}

/// Summarises the files of the ISO at `image_path`: `None` when it has no
/// UDF filesystem, an error message when one is there but cannot be read.
pub fn summarize_iso_files(image_path: &Path) -> Option<Result<IsoFilesSummary, String>> {
    let iso = match File::open(image_path) {
        Ok(iso) => iso,
        Err(error) => return Some(Err(error.to_string())),
    };
    summarize_with_limits(&iso, &CopyLimits::default())
}

pub(crate) fn summarize_with_limits(
    iso: &dyn ImageRead,
    limits: &CopyLimits,
) -> Option<Result<IsoFilesSummary, String>> {
    let volume = match read_udf(iso) {
        Ok(volume) => volume,
        Err(UdfError::NotUdf) => return None,
        Err(error) => return Some(Err(error.to_string())),
    };
    let files = volume.files();
    let mut split_files = Vec::new();
    let mut cannot_copy = Vec::new();
    for (path, file) in files
        .iter()
        .filter(|(_, file)| file.size_bytes > limits.max_file_bytes)
    {
        match split_large_file(iso, path, file, limits) {
            Ok((split_file, _)) => split_files.push(split_file),
            Err(error) => cannot_copy.push(error.to_string()),
        }
    }
    Some(Ok(IsoFilesSummary {
        label: volume.label.clone(),
        file_count: files.len(),
        file_bytes: files.iter().map(|(_, file)| file.size_bytes).sum(),
        largest_file: largest_file(&volume),
        uefi_loader: uefi_loader_path(&volume),
        split_files,
        cannot_copy,
    }))
}

/// Plans copying the files of the ISO at `image_path` onto a drive of
/// `drive_bytes` with `sector_bytes` sectors. `session_id` seeds the volume
/// serial number and disk signature, so they differ between writes.
pub fn lay_out_uefi_fat32(
    image_path: &Path,
    drive_bytes: u64,
    sector_bytes: u32,
    session_id: &str,
    created_unix_seconds: u64,
) -> Result<UefiFat32Layout, FileCopyError> {
    let iso = File::open(image_path).map_err(FileCopyError::Image)?;
    lay_out_with_limits(
        &iso,
        drive_bytes,
        sector_bytes,
        session_id,
        created_unix_seconds,
        &CopyLimits::default(),
    )
}

pub(crate) fn lay_out_with_limits(
    iso: &dyn ImageRead,
    drive_bytes: u64,
    sector_bytes: u32,
    session_id: &str,
    created_unix_seconds: u64,
    limits: &CopyLimits,
) -> Result<UefiFat32Layout, FileCopyError> {
    let volume = read_udf(iso)?;
    let tree = copy_tree(&volume, iso, limits)?;
    let seed = stable_fingerprint(session_id.as_bytes());
    let options = FatImageOptions {
        drive_bytes,
        sector_bytes,
        label: volume.label.clone(),
        volume_id: seed as u32,
        // Zero means "no signature" to Windows, so the low bit is always set.
        disk_signature: ((seed >> 32) as u32) | 1,
        unix_seconds: created_unix_seconds,
    };
    let image = FatImage::build(&tree.root, &options).map_err(FileCopyError::Fat)?;

    Ok(UefiFat32Layout {
        label: image.label().to_string(),
        file_count: tree.sources.len(),
        file_bytes: tree.sources.iter().map(FileSource::size_bytes).sum(),
        largest_file: largest_file(&volume),
        split_files: tree.split_files,
        image_bytes: image.image_len(),
        sector_bytes,
        cluster_bytes: image.cluster_bytes(),
        volume_id: options.volume_id,
        disk_signature: options.disk_signature,
        created_unix_seconds,
    })
}

impl FileSource {
    fn size_bytes(&self) -> u64 {
        match self {
            Self::Udf(file) => file.size_bytes,
            Self::SplitPart { part, .. } => part.size_bytes,
        }
    }
}

fn largest_file(volume: &UdfVolume) -> Option<(String, u64)> {
    volume
        .files()
        .into_iter()
        .max_by_key(|(_, file)| file.size_bytes)
        .map(|(path, file)| (path, file.size_bytes))
}

/// The planned image, built again at write time, with what it needs to
/// generate its bytes.
pub(crate) struct PreparedCopy<R: ImageRead> {
    pub(crate) image: FatImage,
    tree: CopyTree,
    iso: R,
}

impl<R: ImageRead> PreparedCopy<R> {
    pub(crate) fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let content = CopyContent {
            iso: &self.iso,
            tree: &self.tree,
        };
        self.image.read_at(&content, offset, buffer)
    }
}

/// Builds the planned image again from the ISO at `image_path`.
pub(crate) fn prepare_copy(
    image_path: &Path,
    drive_bytes: u64,
    layout: &UefiFat32Layout,
) -> Result<PreparedCopy<File>, FileCopyError> {
    let iso = File::open(image_path).map_err(FileCopyError::Image)?;
    prepare_with_limits(iso, drive_bytes, layout, &CopyLimits::default())
}

pub(crate) fn prepare_with_limits<R: ImageRead>(
    iso: R,
    drive_bytes: u64,
    layout: &UefiFat32Layout,
    limits: &CopyLimits,
) -> Result<PreparedCopy<R>, FileCopyError> {
    let volume = read_udf(&iso)?;
    let tree = copy_tree(&volume, &iso, limits)?;
    let options = FatImageOptions {
        drive_bytes,
        sector_bytes: layout.sector_bytes,
        label: volume.label.clone(),
        volume_id: layout.volume_id,
        disk_signature: layout.disk_signature,
        unix_seconds: layout.created_unix_seconds,
    };
    let image = FatImage::build(&tree.root, &options).map_err(FileCopyError::Fat)?;
    Ok(PreparedCopy { image, tree, iso })
}

/// Reads file contents for the FAT32 image straight from the ISO.
struct CopyContent<'a> {
    iso: &'a dyn ImageRead,
    tree: &'a CopyTree,
}

impl ContentSource for CopyContent<'_> {
    fn read_content(&self, content: usize, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        match &self.tree.sources[content] {
            FileSource::Udf(file) => file.read_at(self.iso, offset, buffer),
            FileSource::SplitPart { wim, part } => {
                let wim = UdfFileImage {
                    iso: self.iso,
                    file: &self.tree.wims[*wim],
                };
                part.read_at(&wim, offset, buffer)
            }
        }
    }
}

/// One file inside the ISO, read as an image of its own.
struct UdfFileImage<'a> {
    iso: &'a dyn ImageRead,
    file: &'a UdfFile,
}

impl ImageRead for UdfFileImage<'_> {
    fn image_len(&self) -> io::Result<u64> {
        Ok(self.file.size_bytes)
    }

    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.file.read_at(self.iso, offset, buffer)
    }
}

/// Plans the parts of a file too large for FAT32, or says why it cannot be
/// copied.
fn split_large_file(
    iso: &dyn ImageRead,
    path: &str,
    file: &UdfFile,
    limits: &CopyLimits,
) -> Result<(SplitFile, Vec<SwmPart>), FileCopyError> {
    if !file.name.eq_ignore_ascii_case(SPLITTABLE_NAME) {
        return Err(FileCopyError::FileTooLarge {
            path: path.to_string(),
            size_bytes: file.size_bytes,
        });
    }
    let image = UdfFileImage { iso, file };
    let split =
        plan_wim_split(&image, limits.part_bytes, limits.max_file_bytes).map_err(|reason| {
            match reason {
                WimSplitError::Io(error) => FileCopyError::Image(error),
                reason => FileCopyError::CannotSplit {
                    path: path.to_string(),
                    reason,
                },
            }
        })?;
    let names = part_names(&file.name, split.parts.len());
    Ok((
        SplitFile {
            path: path.to_string(),
            size_bytes: file.size_bytes,
            parts: names,
        },
        split.parts,
    ))
}

/// Turns the ISO's tree into the FAT32 tree, splitting `install.wim` when
/// it is too large and refusing an ISO with no UEFI boot loader or another
/// file FAT32 cannot hold.
fn copy_tree(
    volume: &UdfVolume,
    iso: &dyn ImageRead,
    limits: &CopyLimits,
) -> Result<CopyTree, FileCopyError> {
    if uefi_loader_path(volume).is_none() {
        return Err(FileCopyError::NoUefiLoader);
    }
    let mut tree = CopyTree {
        root: FatDirectory {
            name: String::new(),
            hidden: false,
            entries: Vec::new(),
        },
        sources: Vec::new(),
        wims: Vec::new(),
        split_files: Vec::new(),
    };
    tree.root = convert_folder(&volume.root, "", iso, limits, &mut tree)?;
    Ok(tree)
}

fn convert_folder(
    folder: &UdfDirectory,
    prefix: &str,
    iso: &dyn ImageRead,
    limits: &CopyLimits,
    tree: &mut CopyTree,
) -> Result<FatDirectory, FileCopyError> {
    let mut entries = Vec::with_capacity(folder.entries.len());
    for entry in &folder.entries {
        match entry {
            UdfEntry::Directory(child) => {
                let path = format!("{prefix}{}/", child.name);
                entries.push(FatEntry::Directory(convert_folder(
                    child, &path, iso, limits, tree,
                )?));
            }
            UdfEntry::File(file) if file.size_bytes <= limits.max_file_bytes => {
                entries.push(FatEntry::File(FatFile {
                    name: file.name.clone(),
                    hidden: file.hidden,
                    size_bytes: file.size_bytes,
                    content: tree.sources.len(),
                }));
                tree.sources.push(FileSource::Udf(file.clone()));
            }
            UdfEntry::File(file) => {
                let path = format!("{prefix}{}", file.name);
                let (split_file, parts) = split_large_file(iso, &path, file, limits)?;
                let wim = tree.wims.len();
                tree.wims.push(file.clone());
                for (name, part) in split_file.parts.iter().zip(parts) {
                    entries.push(FatEntry::File(FatFile {
                        name: name.clone(),
                        hidden: file.hidden,
                        size_bytes: part.size_bytes,
                        content: tree.sources.len(),
                    }));
                    tree.sources.push(FileSource::SplitPart { wim, part });
                }
                tree.split_files.push(split_file);
            }
        }
    }
    Ok(FatDirectory {
        name: folder.name.clone(),
        hidden: folder.hidden,
        entries,
    })
}

/// UEFI firmware starts `\EFI\BOOT\BOOT<architecture>.EFI` from a removable
/// drive, such as `BOOTX64.EFI` or `BOOTAA64.EFI`.
fn uefi_loader_path(volume: &UdfVolume) -> Option<String> {
    let Some(UdfEntry::Directory(folder)) = volume.find("efi/boot") else {
        return None;
    };
    folder
        .entries
        .iter()
        .filter(|entry| matches!(entry, UdfEntry::File(_)))
        .map(UdfEntry::name)
        .find(|name| {
            let name = name.to_ascii_lowercase();
            name.starts_with("boot") && name.ends_with(".efi")
        })
        .map(|name| format!("efi/boot/{name}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        copy_tree, lay_out_uefi_fat32, lay_out_with_limits, prepare_copy, prepare_with_limits,
        summarize_with_limits, CopyLimits, FileCopyError, FileSource,
    };
    use crate::drive_contents::{inspect_drive_contents, FilesystemKind};
    use crate::fat32::FatEntry;
    use crate::image_io::ImageRead;
    use crate::test_support::SparseDrive;
    use crate::udf::{read_udf, DataExtent, FileData, UdfDirectory, UdfEntry, UdfFile, UdfVolume};
    use crate::wim_split::{plan_wim_split, WimSplitError};
    use std::fs;
    use std::path::PathBuf;

    const MIB: u64 = 1024 * 1024;

    fn windows_like() -> SparseDrive {
        SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/isos/windows-like-udf.sparse"
        ))
    }

    fn with_wim() -> SparseDrive {
        SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/wims/windows-with-wim-udf.sparse"
        ))
    }

    /// Small limits so the fixture's 11 KB install.wim counts as too large
    /// and splits into several parts.
    fn small_limits() -> CopyLimits {
        CopyLimits {
            max_file_bytes: 8 * 1024,
            part_bytes: 4 * 1024,
        }
    }

    /// Writes the genisoimage-built Windows-like fixture out as a real file.
    pub(crate) fn windows_like_iso() -> PathBuf {
        let image = windows_like();
        let size = image.image_len().unwrap() as usize;
        let path = crate::test_support::temp_dir("file-copy").join("windows-like.iso");
        fs::write(&path, image.read(0, size)).unwrap();
        path
    }

    fn file(name: &str, size_bytes: u64) -> UdfEntry {
        UdfEntry::File(UdfFile {
            name: name.to_string(),
            hidden: false,
            size_bytes,
            data: FileData::Extents(vec![DataExtent {
                image_offset: None,
                length: size_bytes,
            }]),
        })
    }

    fn folder(name: &str, entries: Vec<UdfEntry>) -> UdfEntry {
        UdfEntry::Directory(UdfDirectory {
            name: name.to_string(),
            hidden: false,
            entries,
        })
    }

    fn volume(entries: Vec<UdfEntry>) -> UdfVolume {
        UdfVolume {
            label: "TEST".to_string(),
            root: UdfDirectory {
                name: String::new(),
                hidden: false,
                entries,
            },
        }
    }

    fn loader() -> UdfEntry {
        folder("efi", vec![folder("boot", vec![file("bootx64.efi", 1)])])
    }

    #[test]
    fn lays_out_a_windows_like_iso() {
        let iso = windows_like_iso();

        let layout = lay_out_uefi_fat32(&iso, 64 * MIB, 512, "OSFLUX-SESSION-1", 1_795_022_016);
        let _ = fs::remove_dir_all(iso.parent().unwrap());
        let layout = layout.unwrap();

        assert_eq!(layout.label, "CCCOMA_X64F");
        assert_eq!(layout.file_count, 48);
        assert!(layout.split_files.is_empty());
        assert_eq!(
            layout.largest_file,
            Some(("sources/boot.wim".to_string(), 3 * MIB))
        );
        assert!(layout.image_bytes > 3 * MIB && layout.image_bytes < 12 * MIB);
        assert_ne!(layout.disk_signature, 0);
    }

    #[test]
    fn builds_the_same_image_again_at_write_time() {
        let iso = windows_like_iso();
        let layout =
            lay_out_uefi_fat32(&iso, 64 * MIB, 512, "OSFLUX-SESSION-1", 1_795_022_016).unwrap();

        let prepared = prepare_copy(&iso, 64 * MIB, &layout);
        let _ = fs::remove_dir_all(iso.parent().unwrap());
        let prepared = prepared.unwrap();

        assert_eq!(prepared.image.image_len(), layout.image_bytes);
        let mut mbr = [0u8; 512];
        prepared.read_at(0, &mut mbr).unwrap();
        assert_eq!(&mbr[440..444], &layout.disk_signature.to_le_bytes());
    }

    #[test]
    fn different_sessions_get_different_disk_signatures() {
        let iso = windows_like();
        let limits = CopyLimits::default();
        let first =
            lay_out_with_limits(&iso, 64 * MIB, 512, "OSFLUX-SESSION-1", 0, &limits).unwrap();
        let second =
            lay_out_with_limits(&iso, 64 * MIB, 512, "OSFLUX-SESSION-2", 0, &limits).unwrap();

        assert_ne!(first.disk_signature, second.disk_signature);
        assert_ne!(first.volume_id, second.volume_id);
    }

    #[test]
    fn splits_install_wim_when_it_is_too_large() {
        let iso = with_wim();

        let layout = lay_out_with_limits(&iso, 64 * MIB, 512, "S", 0, &small_limits()).unwrap();

        assert_eq!(layout.split_files.len(), 1);
        let split = &layout.split_files[0];
        assert_eq!(split.path, "sources/install.wim");
        assert!(split.parts.len() >= 3, "{:?}", split.parts);
        assert_eq!(split.parts[0], "install.swm");
        assert_eq!(split.parts[1], "install2.swm");
    }

    #[test]
    fn writes_the_parts_in_place_of_install_wim() {
        let iso = with_wim();
        let volume = read_udf(&iso).unwrap();

        let tree = copy_tree(&volume, &iso, &small_limits()).unwrap();

        let Some(FatEntry::Directory(sources)) =
            tree.root.entries.iter().find(
                |entry| matches!(entry, FatEntry::Directory(folder) if folder.name == "sources"),
            )
        else {
            panic!("no sources folder");
        };
        let names: Vec<&str> = sources
            .entries
            .iter()
            .map(|entry| match entry {
                FatEntry::File(file) => file.name.as_str(),
                FatEntry::Directory(folder) => folder.name.as_str(),
            })
            .collect();
        assert!(!names.contains(&"install.wim"));
        assert_eq!(names[0], "install.swm");
        assert!(names.iter().all(|name| name.ends_with(".swm")));

        // Each part's bytes are exactly what the splitter generates from the
        // WIM inside the ISO.
        let UdfEntry::File(wim) = volume.find("sources/install.wim").unwrap() else {
            panic!("install.wim is not a file");
        };
        let mut wim_bytes = vec![0u8; wim.size_bytes as usize];
        wim.read_at(&iso, 0, &mut wim_bytes).unwrap();
        let expected = plan_wim_split(&wim_bytes, 4 * 1024, 8 * 1024).unwrap();
        let parts: Vec<_> = tree
            .sources
            .iter()
            .filter_map(|source| match source {
                FileSource::SplitPart { part, .. } => Some(part.clone()),
                FileSource::Udf(_) => None,
            })
            .collect();
        assert_eq!(parts, expected.parts);
    }

    #[test]
    fn a_split_copy_writes_and_reads_back_as_fat32() {
        let iso = with_wim();
        let layout = lay_out_with_limits(&iso, 64 * MIB, 512, "S", 0, &small_limits()).unwrap();
        let prepared = prepare_with_limits(&iso, 64 * MIB, &layout, &small_limits()).unwrap();

        let mut bytes = vec![0u8; layout.image_bytes as usize];
        prepared.read_at(0, &mut bytes).unwrap();
        let drive = SparseDrive::empty(64 * MIB, 0);
        drive.write(0, &bytes);
        let contents = inspect_drive_contents(&drive);

        assert_eq!(
            contents.partitions[0].filesystem,
            Some(FilesystemKind::Fat32)
        );
        assert_eq!(contents.partitions[0].label.as_deref(), Some("CCCOMA_X64F"));
    }

    #[test]
    fn the_split_parts_on_the_stick_install_like_the_original() {
        // mtools and wimlib read the stick as an independent check: the parts
        // copied off the generated FAT32 image must apply to the same files
        // as the original install.wim. Where the tools are missing the test
        // says so and passes.
        for tool in ["mcopy", "wimlib-imagex"] {
            if !crate::test_support::tool_exists(tool) {
                eprintln!("skipping: {tool} is not installed");
                return;
            }
        }
        let iso = with_wim();
        let layout = lay_out_with_limits(&iso, 64 * MIB, 512, "S", 0, &small_limits()).unwrap();
        let prepared = prepare_with_limits(&iso, 64 * MIB, &layout, &small_limits()).unwrap();
        let directory = crate::test_support::temp_dir("split-copy");
        let mut bytes = vec![0u8; layout.image_bytes as usize];
        prepared.read_at(0, &mut bytes).unwrap();
        let volume_path = directory.join("volume.img");
        let volume_file = fs::File::create(&volume_path).unwrap();
        volume_file
            .set_len(prepared.image.partition_bytes())
            .unwrap();
        use std::io::Write as _;
        (&volume_file).write_all(&bytes[MIB as usize..]).unwrap();
        drop(volume_file);

        let volume = read_udf(&iso).unwrap();
        let UdfEntry::File(wim) = volume.find("sources/install.wim").unwrap() else {
            panic!("install.wim is not a file");
        };
        let mut wim_bytes = vec![0u8; wim.size_bytes as usize];
        wim.read_at(&iso, 0, &mut wim_bytes).unwrap();
        fs::write(directory.join("original.wim"), &wim_bytes).unwrap();

        let copied = std::process::Command::new("mcopy")
            .env("MTOOLS_SKIP_CHECK", "1")
            .args(["-n", "-i"])
            .arg(&volume_path)
            .arg("::/sources/*.swm")
            .arg(&directory)
            .output()
            .unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("wimlib-imagex")
                .args(args)
                .current_dir(&directory)
                .output()
                .unwrap()
        };
        let verify = run(&["verify", "install.swm", "--ref=install*.swm"]);
        let from_parts = run(&[
            "apply",
            "install.swm",
            "1",
            "from-parts",
            "--ref=install*.swm",
        ]);
        let from_original = run(&["apply", "original.wim", "1", "from-original"]);
        let parts_tree = crate::test_support::read_tree(&directory.join("from-parts"));
        let original_tree = crate::test_support::read_tree(&directory.join("from-original"));
        let _ = fs::remove_dir_all(&directory);

        assert!(
            copied.status.success(),
            "{}",
            String::from_utf8_lossy(&copied.stderr)
        );
        assert!(
            verify.status.success(),
            "{}{}",
            String::from_utf8_lossy(&verify.stdout),
            String::from_utf8_lossy(&verify.stderr)
        );
        assert!(
            from_parts.status.success(),
            "{}",
            String::from_utf8_lossy(&from_parts.stderr)
        );
        assert!(from_original.status.success());
        assert!(!original_tree.is_empty());
        assert_eq!(parts_tree, original_tree);
    }

    /// Writes the FAT32 partition planned for a real-size ISO to a file, so
    /// it can be checked with fsck.fat, mtools and wimlib. Run it with
    /// `OSFLUX_LARGE_ISO=<iso> OSFLUX_LARGE_VOLUME=<file> cargo test --release
    /// -p osflux-engine large_iso -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a large ISO named by OSFLUX_LARGE_ISO"]
    fn writes_the_volume_of_a_large_iso_to_a_file() {
        use std::io::Write as _;
        let iso = PathBuf::from(std::env::var("OSFLUX_LARGE_ISO").unwrap());
        let volume = PathBuf::from(std::env::var("OSFLUX_LARGE_VOLUME").unwrap());
        let drive_bytes = 16 * 1024 * MIB;

        let layout = lay_out_uefi_fat32(&iso, drive_bytes, 512, "LARGE", 0).unwrap();
        eprintln!("{layout:#?}");
        let prepared = prepare_copy(&iso, drive_bytes, &layout).unwrap();
        assert_eq!(prepared.image.image_len(), layout.image_bytes);

        let start = prepared.image.partition_offset();
        let mut output = std::io::BufWriter::new(fs::File::create(&volume).unwrap());
        let mut buffer = vec![0u8; 4 * MIB as usize];
        let mut offset = start;
        while offset < layout.image_bytes {
            let length = (layout.image_bytes - offset).min(buffer.len() as u64) as usize;
            prepared.read_at(offset, &mut buffer[..length]).unwrap();
            output.write_all(&buffer[..length]).unwrap();
            offset += length as u64;
        }
        let file = output.into_inner().unwrap();
        file.set_len(prepared.image.partition_bytes()).unwrap();
    }

    #[test]
    fn summarizes_a_wim_that_will_be_split() {
        let summary = summarize_with_limits(&with_wim(), &small_limits())
            .unwrap()
            .unwrap();

        assert_eq!(summary.split_files.len(), 1);
        assert!(summary.cannot_copy.is_empty());
        assert_eq!(summary.uefi_loader.as_deref(), Some("efi/boot/bootx64.efi"));
    }

    #[test]
    fn summarizes_the_files_of_a_windows_like_iso() {
        let summary = summarize_with_limits(&windows_like(), &CopyLimits::default())
            .unwrap()
            .unwrap();

        assert_eq!(summary.label, "CCCOMA_X64FRE_EN-US_DV9");
        assert_eq!(summary.file_count, 48);
        assert_eq!(summary.uefi_loader.as_deref(), Some("efi/boot/bootx64.efi"));
        assert!(summary.split_files.is_empty() && summary.cannot_copy.is_empty());
    }

    #[test]
    fn refuses_an_esd_too_large_for_fat32() {
        // An ISO whose only content is a solid image, standing in for an
        // install.esd: read in place through an extent covering it.
        let solid = SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/wims/small-solid.wim.sparse"
        ));
        let size = solid.image_len().unwrap();
        let mut esd = file("install.wim", size);
        if let UdfEntry::File(file) = &mut esd {
            file.data = FileData::Extents(vec![DataExtent {
                image_offset: Some(0),
                length: size,
            }]);
        }
        let iso_volume = volume(vec![loader(), folder("sources", vec![esd])]);

        let error = copy_tree(&iso_volume, &solid, &small_limits())
            .err()
            .unwrap();

        assert!(
            matches!(
                &error,
                FileCopyError::CannotSplit {
                    reason: WimSplitError::Solid,
                    ..
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("solid compression"), "{error}");
    }

    #[test]
    fn refuses_another_file_too_large_for_fat32() {
        let iso_volume = volume(vec![
            loader(),
            folder("sources", vec![file("huge.bin", 5 * 1024 * MIB)]),
        ]);

        let error = copy_tree(&iso_volume, &Vec::new(), &CopyLimits::default())
            .err()
            .unwrap();

        assert!(
            matches!(&error, FileCopyError::FileTooLarge { path, .. } if path == "sources/huge.bin")
        );
        assert!(error.to_string().contains("5.00 GiB"), "{error}");
    }

    #[test]
    fn refuses_an_iso_without_a_uefi_loader() {
        let iso_volume = volume(vec![folder(
            "efi",
            vec![folder("boot", vec![file("readme.txt", 1)])],
        )]);

        assert!(matches!(
            copy_tree(&iso_volume, &Vec::new(), &CopyLimits::default()),
            Err(FileCopyError::NoUefiLoader)
        ));
    }

    #[test]
    fn accepts_any_architecture_loader() {
        for name in ["bootx64.efi", "BOOTAA64.EFI", "bootia32.efi"] {
            let iso_volume = volume(vec![folder(
                "EFI",
                vec![folder("Boot", vec![file(name, 1)])],
            )]);
            assert!(
                copy_tree(&iso_volume, &Vec::new(), &CopyLimits::default()).is_ok(),
                "{name}"
            );
        }
    }

    #[test]
    fn keeps_hidden_flags_and_the_tree_shape() {
        let mut hidden = file("autorun.inf", 10);
        if let UdfEntry::File(file) = &mut hidden {
            file.hidden = true;
        }
        let iso_volume = volume(vec![hidden, loader()]);

        let tree = copy_tree(&iso_volume, &Vec::new(), &CopyLimits::default()).unwrap();

        assert_eq!(tree.sources.len(), 2);
        assert!(
            matches!(&tree.root.entries[0], FatEntry::File(file) if file.hidden && file.content == 0)
        );
        assert!(
            matches!(&tree.root.entries[1], FatEntry::Directory(folder) if folder.name == "efi")
        );
    }

    #[test]
    fn refuses_an_image_without_udf() {
        let plain =
            SparseDrive::from_fixture(include_str!("../tests/fixtures/isos/plain-iso9660.sparse"));

        let error =
            lay_out_with_limits(&plain, 64 * MIB, 512, "S", 0, &CopyLimits::default()).unwrap_err();

        assert!(matches!(error, FileCopyError::NoUdf), "{error}");
    }
}
