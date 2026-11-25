//! OSflux Studio: the desktop app for writing bootable USB drives.
//!
//! Every safety decision is the engine's: Studio shows what the engine
//! reports and passes the operator's choices to it, and never writes to a
//! drive itself.

slint::include_modules!();

pub mod about;
pub mod format;
pub mod settings;
pub mod shell;
pub mod steps;
pub mod system;

#[cfg(feature = "preview")]
pub mod preview;
