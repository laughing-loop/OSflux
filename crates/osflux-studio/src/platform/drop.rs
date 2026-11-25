//! Dropping an image file onto the window.
//!
//! Studio runs as Administrator, and Windows does not let a program running
//! without it (Explorer) drag into one that runs with it through the usual
//! OLE drag and drop. The older WM_DROPFILES message can be let through, so
//! on Windows the window takes drops that way: its OLE drop target is
//! removed, WM_DROPFILES and the two messages that carry its data are
//! allowed in from lower integrity, and the window procedure is wrapped to
//! receive it. The wrapping uses user32 only (not comctl32's subclassing,
//! which needs the Common Controls 6 manifest). Elsewhere, dropping is not
//! supported yet; Browse works.

use std::path::PathBuf;

/// Called with the first dropped path, on the UI thread.
pub type DropHandler = Box<dyn Fn(PathBuf)>;

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, TRUE, WPARAM};
    use windows::Win32::System::Ole::RevokeDragDrop;
    use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, ChangeWindowMessageFilterEx, SetWindowLongPtrW, GWLP_WNDPROC,
        MSGFLT_ALLOW, WM_COPYDATA, WM_DROPFILES, WNDPROC,
    };

    use super::DropHandler;

    /// Carries the data of WM_DROPFILES across integrity levels. Not in
    /// the Windows headers.
    const WM_COPYGLOBALDATA: u32 = 0x0049;

    /// The window procedure that was there before, which every other
    /// message is passed on to.
    static PREVIOUS: AtomicIsize = AtomicIsize::new(0);

    thread_local! {
        static HANDLER: RefCell<Option<DropHandler>> = const { RefCell::new(None) };
    }

    pub fn accept_dropped_files(hwnd: isize, handler: DropHandler) -> Result<(), String> {
        if PREVIOUS.load(Ordering::SeqCst) != 0 {
            return Err("dropped files are already accepted".to_string());
        }
        HANDLER.with(|slot| *slot.borrow_mut() = Some(handler));
        crate::log!("drop: setting up on window {hwnd:#x}");
        let hwnd = HWND(hwnd);
        // SAFETY: `hwnd` is the live window of this thread. The new window
        // procedure handles WM_DROPFILES and passes everything else to the
        // previous one, which is stored before any message can reach it.
        unsafe {
            // Removing winit's OLE target routes drops to WM_DROPFILES.
            match RevokeDragDrop(hwnd) {
                Ok(()) => crate::log!("drop: RevokeDragDrop: removed winit's OLE drop target"),
                Err(error) => crate::log!(
                    "drop: RevokeDragDrop: {:#010x} {}",
                    error.code().0,
                    error.message()
                ),
            }
            let mut allowed = true;
            for (name, message) in [
                ("WM_DROPFILES", WM_DROPFILES),
                ("WM_COPYDATA", WM_COPYDATA),
                ("WM_COPYGLOBALDATA", WM_COPYGLOBALDATA),
            ] {
                match ChangeWindowMessageFilterEx(hwnd, message, MSGFLT_ALLOW, None) {
                    Ok(()) => crate::log!("drop: ChangeWindowMessageFilterEx {name}: allowed"),
                    Err(error) => {
                        allowed = false;
                        crate::log!(
                            "drop: ChangeWindowMessageFilterEx {name}: {:#010x} {}",
                            error.code().0,
                            error.message()
                        );
                    }
                }
            }
            if !allowed {
                return Err("Windows would not let dropped files through".to_string());
            }
            let procedure: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
                window_proc;
            let previous = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, procedure as usize as isize);
            if previous == 0 {
                let error = windows::core::Error::from_win32();
                crate::log!(
                    "drop: SetWindowLongPtrW: failed, {:#010x} {}",
                    error.code().0,
                    error.message()
                );
                return Err("could not receive dropped files".to_string());
            }
            crate::log!(
                "drop: SetWindowLongPtrW: window procedure wrapped (previous {previous:#x})"
            );
            PREVIOUS.store(previous, Ordering::SeqCst);
            DragAcceptFiles(hwnd, TRUE);
            crate::log!("drop: DragAcceptFiles: on; ready for dropped files");
        }
        Ok(())
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message != WM_DROPFILES {
            // SAFETY: PREVIOUS holds the window procedure SetWindowLongPtrW
            // returned, a valid WNDPROC for this window.
            let previous = std::mem::transmute::<isize, WNDPROC>(PREVIOUS.load(Ordering::SeqCst));
            return CallWindowProcW(previous, hwnd, message, wparam, lparam);
        }
        let drop = HDROP(wparam.0 as isize);
        let count = DragQueryFileW(drop, u32::MAX, None);
        crate::log!("drop: WM_DROPFILES received, {count} file(s)");
        let first = (count > 0).then(|| {
            let length = DragQueryFileW(drop, 0, None) as usize;
            let mut buffer = vec![0u16; length + 1];
            let copied = DragQueryFileW(drop, 0, Some(&mut buffer)) as usize;
            PathBuf::from(OsString::from_wide(&buffer[..copied.min(length)]))
        });
        DragFinish(drop);
        if let Some(path) = first {
            crate::log!("drop: opening {}", path.display());
            HANDLER.with(|slot| {
                if let Some(handler) = slot.borrow().as_ref() {
                    handler(path);
                }
            });
        }
        LRESULT(0)
    }
}

/// Lets files be dropped onto the window `hwnd` (a Win32 window handle).
#[cfg(windows)]
pub fn accept_dropped_files(hwnd: isize, handler: DropHandler) -> Result<(), String> {
    imp::accept_dropped_files(hwnd, handler)
}

/// Dropping is not supported here yet.
#[cfg(not(windows))]
pub fn accept_dropped_files(_hwnd: isize, _handler: DropHandler) -> Result<(), String> {
    Ok(())
}
