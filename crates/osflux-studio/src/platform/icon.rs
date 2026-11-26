//! The window's icon in the title bar, the taskbar and Alt+Tab.
//!
//! Slint gives the window one image, scaled from `osflux-256.png`, and only
//! as its small icon. On Windows the window instead takes the icon embedded
//! in `osflux-studio.exe` (`assets/brand/icons/windows/osflux.ico`, built in
//! by `build.rs`) at the sizes Windows asks for: the pixel-tuned 16, 20 and
//! 24 px frames for the title bar, and a large frame for the taskbar and
//! Alt+Tab. Elsewhere Slint's icon stays.

/// Gives the window `handle` the program's own icon at the sizes Windows
/// uses. Does nothing outside Windows.
pub fn use_program_icon(handle: isize) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, GetWindowLongPtrW, LoadImageW, SendMessageW, GWLP_HINSTANCE,
            ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, SM_CYICON,
            SM_CYSMICON, SYSTEM_METRICS_INDEX, WM_SETICON,
        };

        /// The icon's resource ID: winresource's `set_icon` uses 1.
        const ICON_ID: usize = 1;

        let hwnd = HWND(handle);
        // SAFETY: `hwnd` is the live window of this thread. Its module is
        // the program, which holds the icon resource. Each icon is loaded
        // once and kept for the life of the window, as WM_SETICON needs.
        unsafe {
            let module = HINSTANCE(GetWindowLongPtrW(hwnd, GWLP_HINSTANCE));
            let set = |kind: u32, width: SYSTEM_METRICS_INDEX, height: SYSTEM_METRICS_INDEX| {
                let (cx, cy) = (GetSystemMetrics(width), GetSystemMetrics(height));
                let icon = LoadImageW(
                    module,
                    PCWSTR(ICON_ID as *const u16),
                    IMAGE_ICON,
                    cx,
                    cy,
                    LR_DEFAULTCOLOR,
                )
                .map_err(|error| format!("could not load the {cx}x{cy} icon: {error}"))?;
                SendMessageW(hwnd, WM_SETICON, WPARAM(kind as usize), LPARAM(icon.0));
                crate::log!("icon: set the {cx}x{cy} icon on window {handle:#x}");
                Ok::<(), String>(())
            };
            set(ICON_SMALL, SM_CXSMICON, SM_CYSMICON)?;
            set(ICON_BIG, SM_CXICON, SM_CYICON)?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Ok(())
    }
}
