//! OSflux Studio: the desktop app for writing bootable USB drives.
//!
//! Every safety decision is the engine's: Studio shows what the engine
//! reports and passes the operator's choices to it, and never writes to a
//! drive itself.

slint::include_modules!();

pub mod about;
pub mod confirm_step;
pub mod drive_step;
pub mod format;
pub mod image_step;
pub mod log;
pub mod platform;
pub mod renderer;
pub mod safety_step;
pub mod session;
pub mod settings;
pub mod shell;
pub mod steps;
pub mod system;
pub mod windows_options;
pub mod write_step;

#[cfg(feature = "preview")]
pub mod preview;
