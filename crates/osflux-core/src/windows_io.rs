use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::WriteFile;
use windows::Win32::System::IO::OVERLAPPED;

pub fn write_overlapped(handle: HANDLE, buffer: &[u8], offset: u64) {
    let mut overlapped = OVERLAPPED::default();

    overlapped.Anonymous.Anonymous.Offset = offset as u32;
    overlapped.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;

    let mut written = 0;

    unsafe {
        let _ = WriteFile(
            handle,
            Some(buffer),
            Some(&mut written),
            Some(&mut overlapped),
        );
    }
}
