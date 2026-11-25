//! What Studio needs from the operating system beyond Slint: the Open
//! dialog, dropped files and the local time.

pub mod clock;
pub mod drop;
pub mod picker;

/// The Win32 handle of a shown window, for the Open dialog's owner and for
/// dropped files. `None` elsewhere, or before the window is shown.
pub fn native_handle(window: &slint::Window) -> Option<isize> {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let handle = window.window_handle();
        match HasWindowHandle::window_handle(&handle).ok()?.as_raw() {
            RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}
