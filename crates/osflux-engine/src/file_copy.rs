//! Copies the files of a UDF ISO, such as a Windows installer, onto a new
//! MBR drive with one FAT32 partition that starts in UEFI mode.
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
use crate::udf::{read_udf_file, UdfDirectory, UdfEntry, UdfError, UdfFile, UdfVolume};

/// What copying the ISO's files will put on the drive, recorded in the write
/// plan so the same image can be built again at write time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UefiFat32Layout {
    /// The FAT32 volume label, from the ISO's label cut to 11 characters.
    pub label: String,
    pub file_count: usize,
    pub file_bytes: u64,
    /// The largest file and its size, for the summary shown before writing.
    pub largest_file: Option<(String, u64)>,
    /// Bytes of the drive image written, from the MBR to the end of the last
    /// file. The free space after it is not written.
    pub image_bytes: u64,
    pub sector_bytes: u32,
    pub cluster_bytes: u64,
    pub volume_id: u32,
    pub disk_signature: u32,
    pub created_unix_seconds: u64,
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
                "{path} is {:.2} GiB, and FAT32 cannot hold a file of 4 GiB or more; splitting it is the next step of OSflux's Windows support and not available yet",
                *size_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
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
    let volume = read_udf_file(image_path)?;
    let (tree, files) = fat_tree_from_udf(&volume)?;
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
    let image = FatImage::build(&tree, &options).map_err(FileCopyError::Fat)?;

    let largest_file = volume
        .files()
        .into_iter()
        .max_by_key(|(_, file)| file.size_bytes)
        .map(|(path, file)| (path, file.size_bytes));
    Ok(UefiFat32Layout {
        label: image.label().to_string(),
        file_count: files.len(),
        file_bytes: files.iter().map(|file| file.size_bytes).sum(),
        largest_file,
        image_bytes: image.image_len(),
        sector_bytes,
        cluster_bytes: image.cluster_bytes(),
        volume_id: options.volume_id,
        disk_signature: options.disk_signature,
        created_unix_seconds,
    })
}

/// The planned image, built again at write time, with what it needs to
/// generate its bytes.
pub(crate) struct PreparedCopy {
    pub(crate) image: FatImage,
    files: Vec<UdfFile>,
    iso: File,
}

impl PreparedCopy {
    pub(crate) fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let content = UdfContent {
            iso: &self.iso,
            files: &self.files,
        };
        self.image.read_at(&content, offset, buffer)
    }
}

/// Builds the planned image again. `None` from the size check means the ISO
/// no longer produces the planned image.
pub(crate) fn prepare_copy(
    image_path: &Path,
    drive_bytes: u64,
    layout: &UefiFat32Layout,
) -> Result<PreparedCopy, FileCopyError> {
    let volume = read_udf_file(image_path)?;
    let (tree, files) = fat_tree_from_udf(&volume)?;
    let options = FatImageOptions {
        drive_bytes,
        sector_bytes: layout.sector_bytes,
        label: volume.label.clone(),
        volume_id: layout.volume_id,
        disk_signature: layout.disk_signature,
        unix_seconds: layout.created_unix_seconds,
    };
    let image = FatImage::build(&tree, &options).map_err(FileCopyError::Fat)?;
    let iso = File::open(image_path).map_err(FileCopyError::Image)?;
    Ok(PreparedCopy { image, files, iso })
}

/// Reads file contents for the FAT32 image straight from the ISO.
struct UdfContent<'a> {
    iso: &'a File,
    files: &'a [UdfFile],
}

impl ContentSource for UdfContent<'_> {
    fn read_content(&self, content: usize, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.files[content].read_at(self.iso, offset, buffer)
    }
}

/// Turns the ISO's tree into the FAT32 tree, refusing an ISO that has no
/// UEFI boot loader or holds a file FAT32 cannot.
pub(crate) fn fat_tree_from_udf(
    volume: &UdfVolume,
) -> Result<(FatDirectory, Vec<UdfFile>), FileCopyError> {
    if !has_uefi_loader(volume) {
        return Err(FileCopyError::NoUefiLoader);
    }
    if let Some((path, file)) = volume
        .files()
        .into_iter()
        .find(|(_, file)| file.size_bytes > MAX_FAT32_FILE_BYTES)
    {
        return Err(FileCopyError::FileTooLarge {
            path,
            size_bytes: file.size_bytes,
        });
    }

    let mut files = Vec::new();
    let root = convert_folder(&volume.root, &mut files);
    Ok((root, files))
}

fn convert_folder(folder: &UdfDirectory, files: &mut Vec<UdfFile>) -> FatDirectory {
    let entries = folder
        .entries
        .iter()
        .map(|entry| match entry {
            UdfEntry::Directory(child) => FatEntry::Directory(convert_folder(child, files)),
            UdfEntry::File(file) => {
                let content = files.len();
                files.push(file.clone());
                FatEntry::File(FatFile {
                    name: file.name.clone(),
                    hidden: file.hidden,
                    size_bytes: file.size_bytes,
                    content,
                })
            }
        })
        .collect();
    FatDirectory {
        name: folder.name.clone(),
        hidden: folder.hidden,
        entries,
    }
}

/// UEFI firmware starts `\EFI\BOOT\BOOT<architecture>.EFI` from a removable
/// drive, such as `BOOTX64.EFI` or `BOOTAA64.EFI`.
fn has_uefi_loader(volume: &UdfVolume) -> bool {
    matches!(
        volume.find("efi/boot"),
        Some(UdfEntry::Directory(folder)) if folder.entries.iter().any(|entry| {
            let name = entry.name().to_ascii_lowercase();
            matches!(entry, UdfEntry::File(_)) && name.starts_with("boot") && name.ends_with(".efi")
        })
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{fat_tree_from_udf, lay_out_uefi_fat32, prepare_copy, FileCopyError};
    use crate::fat32::FatEntry;
    use crate::test_support::SparseDrive;
    use crate::udf::{DataExtent, FileData, UdfDirectory, UdfEntry, UdfFile, UdfVolume};
    use std::fs;
    use std::path::PathBuf;

    const MIB: u64 = 1024 * 1024;

    /// Writes the genisoimage-built Windows-like fixture out as a real file.
    pub(crate) fn windows_like_iso() -> PathBuf {
        let image = SparseDrive::from_fixture(include_str!(
            "../tests/fixtures/isos/windows-like-udf.sparse"
        ));
        let size = crate::image_io::ImageRead::image_len(&image).unwrap() as usize;
        let path = std::env::temp_dir().join(format!(
            "osflux-file-copy-test-{}-{}.iso",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
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

    #[test]
    fn lays_out_a_windows_like_iso() {
        let iso = windows_like_iso();

        let layout = lay_out_uefi_fat32(&iso, 64 * MIB, 512, "OSFLUX-SESSION-1", 1_795_022_016);
        let _ = fs::remove_file(&iso);
        let layout = layout.unwrap();

        assert_eq!(layout.label, "CCCOMA_X64F");
        assert_eq!(layout.file_count, 48);
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
        let _ = fs::remove_file(&iso);
        let prepared = prepared.unwrap();

        assert_eq!(prepared.image.image_len(), layout.image_bytes);
        let mut mbr = [0u8; 512];
        prepared.read_at(0, &mut mbr).unwrap();
        assert_eq!(&mbr[440..444], &layout.disk_signature.to_le_bytes());
    }

    #[test]
    fn different_sessions_get_different_disk_signatures() {
        let iso = windows_like_iso();
        let first = lay_out_uefi_fat32(&iso, 64 * MIB, 512, "OSFLUX-SESSION-1", 0).unwrap();
        let second = lay_out_uefi_fat32(&iso, 64 * MIB, 512, "OSFLUX-SESSION-2", 0).unwrap();
        let _ = fs::remove_file(&iso);

        assert_ne!(first.disk_signature, second.disk_signature);
        assert_ne!(first.volume_id, second.volume_id);
    }

    #[test]
    fn refuses_an_iso_without_a_uefi_loader() {
        let iso = volume(vec![folder(
            "efi",
            vec![folder("boot", vec![file("readme.txt", 1)])],
        )]);

        assert!(matches!(
            fat_tree_from_udf(&iso),
            Err(FileCopyError::NoUefiLoader)
        ));
    }

    #[test]
    fn accepts_any_architecture_loader() {
        for loader in ["bootx64.efi", "BOOTAA64.EFI", "bootia32.efi"] {
            let iso = volume(vec![folder(
                "EFI",
                vec![folder("Boot", vec![file(loader, 1)])],
            )]);
            assert!(fat_tree_from_udf(&iso).is_ok(), "{loader}");
        }
    }

    #[test]
    fn names_the_file_that_is_too_large_for_fat32() {
        let iso = volume(vec![
            folder("efi", vec![folder("boot", vec![file("bootx64.efi", 1)])]),
            folder("sources", vec![file("install.wim", 5 * 1024 * MIB)]),
        ]);

        let error = fat_tree_from_udf(&iso).unwrap_err();

        assert!(
            matches!(&error, FileCopyError::FileTooLarge { path, .. } if path == "sources/install.wim")
        );
        assert!(error.to_string().contains("5.00 GiB"), "{error}");
    }

    #[test]
    fn keeps_hidden_flags_and_the_tree_shape() {
        let mut hidden = file("autorun.inf", 10);
        if let UdfEntry::File(file) = &mut hidden {
            file.hidden = true;
        }
        let iso = volume(vec![
            hidden,
            folder("efi", vec![folder("boot", vec![file("bootx64.efi", 1)])]),
        ]);

        let (tree, files) = fat_tree_from_udf(&iso).unwrap();

        assert_eq!(files.len(), 2);
        assert!(
            matches!(&tree.entries[0], FatEntry::File(file) if file.hidden && file.content == 0)
        );
        assert!(matches!(&tree.entries[1], FatEntry::Directory(folder) if folder.name == "efi"));
    }

    #[test]
    fn refuses_an_image_without_udf() {
        let plain =
            SparseDrive::from_fixture(include_str!("../tests/fixtures/isos/plain-iso9660.sparse"));
        let size = crate::image_io::ImageRead::image_len(&plain).unwrap() as usize;
        let path = std::env::temp_dir().join(format!("osflux-plain-{}.iso", std::process::id()));
        fs::write(&path, plain.read(0, size)).unwrap();

        let error = lay_out_uefi_fat32(&path, 64 * MIB, 512, "S", 0).unwrap_err();
        let _ = fs::remove_file(&path);

        assert!(matches!(error, FileCopyError::NoUdf), "{error}");
    }
}
