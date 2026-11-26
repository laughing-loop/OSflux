//! What Studio needs from the operating system beyond Slint: the Open
//! dialog, dropped files and the local time.

pub mod clock;
pub mod drop;
pub mod elevate;
pub mod icon;
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

/// Whether the window `handle` is visible and has been painted: Windows has
/// no part of it waiting to be drawn. The software renderer cannot report
/// its frames, so this is how Studio logs its first one. `None` elsewhere.
pub fn window_painted(handle: isize) -> Option<bool> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{FALSE, HWND};
        use windows::Win32::Graphics::Gdi::GetUpdateRect;
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        let hwnd = HWND(handle);
        // SAFETY: both only read the state of a window of this thread.
        unsafe {
            Some(IsWindowVisible(hwnd).as_bool() && !GetUpdateRect(hwnd, None, FALSE).as_bool())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        None
    }
}
