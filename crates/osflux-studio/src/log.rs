//! Studio's own log, `studio.log` in OSflux's state folder
//! (`%LOCALAPPDATA%\OSflux` on Windows), for problems nobody sees: the
//! release exe has no console, so messages printed there go nowhere.
//!
//! Each launch starts the file afresh and keeps the one before as
//! `studio.previous.log`. Lines carry the milliseconds since the process
//! started, so slow steps at launch stand out. Messages are also printed to
//! the console, for debug builds. Nothing here records what is on a drive or
//! in an image beyond the file names and drive paths Studio shows on screen.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const FILE_NAME: &str = "studio.log";
const PREVIOUS_FILE_NAME: &str = "studio.previous.log";

struct Log {
    started: Instant,
    /// Milliseconds between the process starting and `init`, when known.
    before_main_ms: u64,
    file: Mutex<Option<File>>,
}

static LOG: OnceLock<Log> = OnceLock::new();

/// Opens the log. Call first thing in `main`; later calls do nothing.
/// Logging still prints to the console when the file cannot be opened.
pub fn init() {
    LOG.get_or_init(|| {
        let started = Instant::now();
        let file = path().and_then(|path| {
            if let Some(folder) = path.parent() {
                fs::create_dir_all(folder).ok()?;
                let _ = fs::rename(&path, folder.join(PREVIOUS_FILE_NAME));
            }
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&path)
                .ok()
        });
        Log {
            started,
            before_main_ms: milliseconds_since_process_start().unwrap_or(0),
            file: Mutex::new(file),
        }
    });
    line(&format!(
        "OSflux Studio {} started; the process started {} ms before main",
        env!("CARGO_PKG_VERSION"),
        LOG.get().map(|log| log.before_main_ms).unwrap_or(0)
    ));
}

/// Where the log is written: `studio.log` in OSflux's state folder.
pub fn path() -> Option<PathBuf> {
    osflux_engine::state_directory().map(|folder| folder.join(FILE_NAME))
}

/// Writes one line, with the time since the process started.
pub fn line(message: &str) {
    eprintln!("{message}");
    let Some(log) = LOG.get() else {
        return;
    };
    let elapsed = log.before_main_ms + log.started.elapsed().as_millis() as u64;
    if let Ok(mut file) = log.file.lock() {
        if let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{elapsed:>7} ms  {message}");
            let _ = file.flush();
        }
    }
}

/// How long ago Windows created this process, so the log shows the time
/// spent loading before `main` too.
#[cfg(windows)]
fn milliseconds_since_process_start() -> Option<u64> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    let to_ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: GetProcessTimes and GetSystemTimeAsFileTime only fill in the
    // structures passed to them, for the current process.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .ok()?;
        let now = GetSystemTimeAsFileTime();
        // FILETIME counts 100-nanosecond intervals.
        to_ticks(now)
            .checked_sub(to_ticks(created))
            .map(|ticks| ticks / 10_000)
    }
}

#[cfg(not(windows))]
fn milliseconds_since_process_start() -> Option<u64> {
    None
}

/// Logs a formatted line: `log!("drop: handle {handle:#x}")`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::line(&format!($($arg)*))
    };
}
