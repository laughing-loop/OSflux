//! The system's Open dialog, for choosing an image. Only reads a path; the
//! file itself is read by the engine afterwards.

use std::path::PathBuf;

/// Shows the Open dialog over the window `owner` (a Win32 window handle)
/// and waits for it to close. `Ok(None)` when the person cancels. The
/// dialog is modal, so this runs on the UI thread.
#[cfg(windows)]
pub fn choose_image(owner: Option<isize>) -> Result<Option<PathBuf>, String> {
    use windows::core::w;
    use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
        SIGDN_FILESYSPATH,
    };

    // SAFETY: plain COM calls on this thread. Strings passed in are static,
    // and the path returned by GetDisplayName is copied before it is freed.
    unsafe {
        // The window's thread already uses COM; this only makes sure.
        let initialised = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let result = (|| -> windows::core::Result<Option<PathBuf>> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetTitle(w!("Choose an image"))?;
            dialog.SetFileTypes(&[
                COMDLG_FILTERSPEC {
                    pszName: w!("Disk images (*.iso, *.img, *.raw)"),
                    pszSpec: w!("*.iso;*.img;*.raw"),
                },
                COMDLG_FILTERSPEC {
                    pszName: w!("All files"),
                    pszSpec: w!("*.*"),
                },
            ])?;
            let options = dialog.GetOptions()?;
            dialog.SetOptions(
                options | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_FORCEFILESYSTEM,
            )?;
            if let Err(error) = dialog.Show(HWND(owner.unwrap_or(0))) {
                if error.code() == ERROR_CANCELLED.to_hresult() {
                    return Ok(None);
                }
                return Err(error);
            }
            let item = dialog.GetResult()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            Ok(Some(PathBuf::from(path.map_err(|_| {
                windows::core::Error::from(windows::Win32::Foundation::E_UNEXPECTED)
            })?)))
        })();
        if initialised.is_ok() {
            CoUninitialize();
        }
        result.map_err(|error| format!("the Open dialog failed: {}", error.message()))
    }
}

/// Asks zenity, or else kdialog, for a file. Blocks until it closes, so it
/// runs off the UI thread. `Ok(None)` when the person cancels.
#[cfg(not(windows))]
pub fn choose_image(_owner: Option<isize>) -> Result<Option<PathBuf>, String> {
    use std::io::ErrorKind;
    use std::process::Command;

    let attempts: [(&str, &[&str]); 2] = [
        (
            "zenity",
            &[
                "--file-selection",
                "--title=Choose an image",
                "--file-filter=Disk images | *.iso *.img *.raw",
                "--file-filter=All files | *",
            ],
        ),
        (
            "kdialog",
            &[
                "--title",
                "Choose an image",
                "--getopenfilename",
                ".",
                "*.iso *.img *.raw|Disk images",
            ],
        ),
    ];
    for (program, args) in attempts {
        match Command::new(program).args(args).output() {
            Ok(output) => {
                if !output.status.success() {
                    // Both exit with 1 when the dialog is cancelled.
                    return Ok(None);
                }
                let path = String::from_utf8_lossy(&output.stdout)
                    .trim_end_matches(['\n', '\r'])
                    .to_string();
                return Ok((!path.is_empty()).then(|| PathBuf::from(path)));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{program} could not be started: {error}")),
        }
    }
    Err("no file chooser was found. Install zenity or kdialog to browse for an image".to_string())
}
