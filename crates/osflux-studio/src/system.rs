//! Handing links and files to the desktop.
//!
//! Studio runs as Administrator, so it asks Explorer to open them: Explorer
//! passes the request to the signed-in user's shell, and the browser does
//! not start with Administrator rights.

use std::io;
use std::path::Path;
use std::process::Command;

/// Opens a web page in the default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    if cfg!(windows) {
        Command::new("explorer.exe").arg(url).spawn().map(|_| ())
    } else {
        Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}

/// Shows a file in the file manager, selected; its folder when the file
/// does not exist yet.
pub fn show_file(path: &Path) -> io::Result<()> {
    if cfg!(windows) {
        let mut command = Command::new("explorer.exe");
        if path.exists() {
            command.arg(format!("/select,{}", path.display()));
        } else if let Some(folder) = path.parent() {
            command.arg(folder);
        }
        command.spawn().map(|_| ())
    } else {
        // xdg-open cannot select a file, so it opens the folder.
        let folder = path.parent().unwrap_or(path);
        Command::new("xdg-open").arg(folder).spawn().map(|_| ())
    }
}
