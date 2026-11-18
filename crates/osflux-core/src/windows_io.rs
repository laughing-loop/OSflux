use std::ffi::c_void;

use windows::core::{Error, Result, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_DATA, ERROR_IO_PENDING, HANDLE};
use windows::Win32::Storage::FileSystem::WriteFile;
use windows::Win32::System::Threading::CreateEventW;
use windows::Win32::System::IO::{DeviceIoControl, GetOverlappedResult, OVERLAPPED};

/// Writes `buffer` at `offset` on a handle opened with `FILE_FLAG_OVERLAPPED`
/// and waits for the write to finish. Windows keeps using the `OVERLAPPED`
/// structure and the buffer until the write completes, so this must not
/// return while the write is still in flight.
pub fn write_overlapped(handle: HANDLE, buffer: &[u8], offset: u64) -> Result<usize> {
    let mut operation = OverlappedOperation::new(offset)?;
    let started = unsafe { WriteFile(handle, Some(buffer), None, Some(operation.as_mut_ptr())) };

    operation
        .wait(handle, started)
        .map(|written| written as usize)
}

/// Sends a control code that fills in a `T` on a handle opened with
/// `FILE_FLAG_OVERLAPPED`. Such handles need an `OVERLAPPED` structure for
/// every request, or the request can return before it has finished.
pub fn query_overlapped<T: Copy + Default>(handle: HANDLE, control_code: u32) -> Result<T> {
    let mut output = T::default();
    let mut operation = OverlappedOperation::new(0)?;
    let started = unsafe {
        DeviceIoControl(
            handle,
            control_code,
            None,
            0,
            Some((&mut output as *mut T).cast::<c_void>()),
            std::mem::size_of::<T>() as u32,
            None,
            Some(operation.as_mut_ptr()),
        )
    };

    let returned = operation.wait(handle, started)? as usize;
    if returned < std::mem::size_of::<T>() {
        return Err(Error::new(
            ERROR_INVALID_DATA.to_hresult(),
            format!(
                "control code {control_code:#x} returned {returned} of {} bytes",
                std::mem::size_of::<T>()
            ),
        ));
    }

    Ok(output)
}

/// Sends a control code without input or output on a handle opened with
/// `FILE_FLAG_OVERLAPPED` and waits for it to finish.
pub fn control_overlapped(handle: HANDLE, control_code: u32) -> Result<()> {
    let mut operation = OverlappedOperation::new(0)?;
    let started = unsafe {
        DeviceIoControl(
            handle,
            control_code,
            None,
            0,
            None,
            0,
            None,
            Some(operation.as_mut_ptr()),
        )
    };

    operation.wait(handle, started).map(|_| ())
}

struct OverlappedOperation {
    overlapped: OVERLAPPED,
}

impl OverlappedOperation {
    fn new(offset: u64) -> Result<Self> {
        let mut overlapped = OVERLAPPED::default();
        overlapped.Anonymous.Anonymous.Offset = offset as u32;
        overlapped.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
        // A separate event per operation means the wait cannot be satisfied
        // by a different request finishing on the same handle.
        overlapped.hEvent = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }?;

        Ok(Self { overlapped })
    }

    fn as_mut_ptr(&mut self) -> *mut OVERLAPPED {
        &mut self.overlapped
    }

    /// Waits for an operation that `started` reports as completed or pending.
    /// Any other error means Windows never queued it, so there is nothing to
    /// wait for.
    fn wait(&mut self, handle: HANDLE, started: Result<()>) -> Result<u32> {
        if let Err(error) = started {
            if error.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(error);
            }
        }

        let mut transferred = 0u32;
        unsafe { GetOverlappedResult(handle, &self.overlapped, &mut transferred, true) }?;
        Ok(transferred)
    }
}

impl Drop for OverlappedOperation {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.overlapped.hEvent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::write_overlapped;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_READ,
    };

    #[test]
    fn writes_at_offsets_and_reports_full_byte_counts() {
        let path = std::env::temp_dir().join(format!(
            "osflux-overlapped-test-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path_utf16: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path_utf16.as_ptr()),
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ,
                None,
                CREATE_ALWAYS,
                FILE_FLAG_OVERLAPPED | FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .unwrap();

        let first = vec![0xABu8; 8 * 1024 * 1024];
        let second = vec![0xCDu8; 4096];
        let second_offset = 16 * 1024 * 1024;

        let first_written = write_overlapped(handle, &first, 0);
        let second_written = write_overlapped(handle, &second, second_offset);
        unsafe {
            let _ = CloseHandle(handle);
        }

        let contents = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(first_written.unwrap(), first.len());
        assert_eq!(second_written.unwrap(), second.len());
        assert_eq!(contents.len() as u64, second_offset + second.len() as u64);
        assert!(contents[..first.len()].iter().all(|byte| *byte == 0xAB));
        assert!(contents[second_offset as usize..]
            .iter()
            .all(|byte| *byte == 0xCD));
    }
}
