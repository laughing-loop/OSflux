//! Reading an image file at any offset, without moving a shared cursor.

use std::fs::File;
use std::io;

/// An image that can be read at any offset. Implemented for files, and for
/// in-memory images in tests.
pub trait ImageRead {
    fn image_len(&self) -> io::Result<u64>;

    /// Fills `buffer` from `offset`, or fails; a short read is an error.
    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()>;
}

impl ImageRead for File {
    fn image_len(&self) -> io::Result<u64> {
        self.metadata().map(|metadata| metadata.len())
    }

    #[cfg(unix)]
    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        use std::os::unix::fs::FileExt;
        self.read_exact_at(buffer, offset)
    }

    #[cfg(windows)]
    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buffer.len() {
            let read = self.seek_read(&mut buffer[done..], offset + done as u64)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!("the image ends before byte {}", offset + done as u64),
                ));
            }
            done += read;
        }
        Ok(())
    }
}

impl ImageRead for [u8] {
    fn image_len(&self) -> io::Result<u64> {
        Ok(self.len() as u64)
    }

    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| past_end(offset))?;
        let end = start
            .checked_add(buffer.len())
            .filter(|end| *end <= self.len())
            .ok_or_else(|| past_end(offset))?;
        buffer.copy_from_slice(&self[start..end]);
        Ok(())
    }
}

impl ImageRead for Vec<u8> {
    fn image_len(&self) -> io::Result<u64> {
        self.as_slice().image_len()
    }

    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.as_slice().read_image_at(offset, buffer)
    }
}

impl<T: ImageRead + ?Sized> ImageRead for &T {
    fn image_len(&self) -> io::Result<u64> {
        (**self).image_len()
    }

    fn read_image_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        (**self).read_image_at(offset, buffer)
    }
}

fn past_end(offset: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("read at byte {offset} runs past the end of the image"),
    )
}
