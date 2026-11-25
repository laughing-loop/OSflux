// A window, not a console, in release builds. Debug builds keep the console
// for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use osflux_studio::settings::Settings;
use osflux_studio::shell::Shell;
use osflux_studio::AppWindow;
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(feature = "preview")]
    if let Some(options) = osflux_studio::preview::Options::from_args(std::env::args().skip(1)) {
        return osflux_studio::preview::run(options);
    }

    let settings_path = Settings::default_path();
    let settings = settings_path
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();

    let window = AppWindow::new()?;
    let shell = Shell::install(
        &window,
        settings,
        settings_path,
        osflux_engine::is_elevated(),
    );
    window.show()?;
    shell.borrow().accept_dropped_files(&window);
    slint::run_event_loop()?;
    window.hide()
}
