//! Whether OSflux runs with the rights it needs to write to a whole drive:
//! an elevated (Administrator) token on Windows, root elsewhere.
//!
//! This only says what to show; the device layer still decides, by whether
//! a drive opens for writing.

/// True when this process is elevated.
pub fn is_elevated() -> bool {
    imp::is_elevated()
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::mem::size_of;

    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    pub fn is_elevated() -> bool {
        let mut token = HANDLE::default();
        // SAFETY: the pseudo-handle from GetCurrentProcess needs no closing,
        // and `token` is a valid place for the opened handle.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        // SAFETY: the buffer is a TOKEN_ELEVATION of the size passed, and
        // `token` is the handle opened above.
        let queried = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut TOKEN_ELEVATION as *mut c_void),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        }
        .is_ok();
        // SAFETY: `token` was opened above and is closed once.
        let _ = unsafe { CloseHandle(token) };
        queried && elevation.TokenIsElevated != 0
    }
}

#[cfg(target_os = "linux")]
mod imp {
    /// Root, read from the effective user id in `/proc/self/status`.
    pub fn is_elevated() -> bool {
        std::fs::read_to_string("/proc/self/status")
            .map(|status| effective_uid_is_root(&status))
            .unwrap_or(false)
    }

    pub(super) fn effective_uid_is_root(status: &str) -> bool {
        status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .and_then(|ids| ids.split_whitespace().nth(1))
            == Some("0")
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub fn is_elevated() -> bool {
        false
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::imp::effective_uid_is_root;

    #[test]
    fn reads_the_effective_user_id() {
        assert!(effective_uid_is_root("Name:\tx\nUid:\t1000\t0\t0\t0\n"));
        assert!(!effective_uid_is_root("Uid:\t0\t1000\t1000\t1000\n"));
        assert!(!effective_uid_is_root("Name:\tx\n"));
    }
}
