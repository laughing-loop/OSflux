//! OSflux Studio: the desktop app for writing bootable USB drives.
//!
//! Every safety decision is the engine's: Studio shows what the engine
//! reports and passes the operator's choices to it, and never writes to a
//! drive itself.

// A window, not a console, in release builds. Debug builds keep the console
// for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

#[cfg(feature = "preview")]
mod preview;

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(feature = "preview")]
    if let Some(options) = preview::Options::from_args(std::env::args().skip(1)) {
        return preview::run(options);
    }

    let window = AppWindow::new()?;
    window.run()
}
