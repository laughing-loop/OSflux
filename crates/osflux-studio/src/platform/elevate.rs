//! "Restart as Administrator" (frame 23): starts Studio again through
//! Windows' "runas" verb, which shows the UAC prompt. The person answers it;
//! Studio only asks. Elsewhere there is nothing to restart into.

/// Starts a new, elevated Studio. Returns once Windows has started it, or
/// with the reason it could not, including the person declining the prompt.
pub fn restart_as_administrator() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        use windows::core::{w, PCWSTR};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let result = unsafe {
            ShellExecuteW(
                HWND(0),
                w!("runas"),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute reports success with a value above 32.
        if result.0 > 32 {
            Ok(())
        } else {
            Err(format!(
                "Windows did not start OSflux as Administrator (code {})",
                result.0
            ))
        }
    }
    #[cfg(not(windows))]
    {
        Err("restarting as Administrator is only available on Windows".to_string())
    }
}
