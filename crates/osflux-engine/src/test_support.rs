//! Helpers shared by the unit tests.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;

use osflux_core::device::{DeviceError, DeviceRead, DeviceResult};

use crate::image_io::ImageRead;

/// A drive or image kept as its non-empty 512-byte sectors, loaded from the
/// fixtures under `tests/fixtures`, which the scripts there build with the
/// standard Linux tools. As a drive it insists on whole-sector reads, like a
/// real one; as an image it can be read at any offset.
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
        let mut sectors = self.sectors.borrow_mut();
        for (index, byte) in bytes.iter().enumerate() {
            let position = offset + index as u64;
            let sector = position - position % 512;
            let data = sectors
                .entry(sector)
                .or_insert_with(|| vec![self.fill; 512]);
            data[(position - sector) as usize] = *byte;
        }
    }

    pub(crate) fn read(&self, offset: u64, length: usize) -> Vec<u8> {
        let mut buffer = vec![0u8; length];
        self.fill_from(offset, &mut buffer);
        buffer
    }

    fn fill_from(&self, offset: u64, buffer: &mut [u8]) {
        assert!(
            offset + buffer.len() as u64 <= self.size,
            "read past the end"
        );
        buffer.fill(self.fill);
        let first_sector = offset - offset % 512;
        let end = offset + buffer.len() as u64;
        let sectors = self.sectors.borrow();
        for (sector, data) in sectors.range(first_sector..end) {
            let start = (*sector).max(offset);
            let stop = (sector + 512).min(end);
            let source = &data[(start - sector) as usize..(stop - sector) as usize];
            buffer[(start - offset) as usize..(stop - offset) as usize].copy_from_slice(source);
        }
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
        self.fill_from(offset, buffer);
        Ok(())
    }

    fn size_bytes(&self) -> DeviceResult<u64> {
        Ok(self.size)
    }

    fn sector_size_bytes(&self) -> DeviceResult<u32> {
        Ok(512)
    }
}

impl ImageRead for SparseDrive {
    fn image_len(&self) -> io::Result<u64> {
        Ok(self.size)
    }

    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        if self.fail_reads {
            return Err(io::Error::other("the image is not readable"));
        }
        if offset + buffer.len() as u64 > self.size {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past the end",
            ));
        }
        self.fill_from(offset, buffer);
        Ok(())
    }
}

/// Whether a program is on the PATH, for tests that use standard tools as
/// an independent check and skip themselves where those are missing.
pub(crate) fn tool_exists(tool: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(tool).is_file())
    })
}

/// A fresh, empty folder in the system temp folder.
pub(crate) fn temp_dir(label: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "osflux-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

/// Every file under `root`, by path relative to it, with its contents.
pub(crate) fn read_tree(root: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(
        root: &std::path::Path,
        folder: &std::path::Path,
        files: &mut BTreeMap<String, Vec<u8>>,
    ) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, files);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files);
    files
}
