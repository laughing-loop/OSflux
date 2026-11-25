// A window, not a console, in release builds. Debug builds keep the console
// for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use osflux_studio::log;
use osflux_studio::settings::Settings;
use osflux_studio::shell::Shell;
use osflux_studio::AppWindow;
use slint::ComponentHandle;

fn main() -> Result<(), slint::PlatformError> {
    #[cfg(feature = "preview")]
    if let Some(options) = osflux_studio::preview::Options::from_args(std::env::args().skip(1)) {
        return osflux_studio::preview::run(options);
    }

    osflux_studio::log::init();

    let settings_path = Settings::default_path();
    let settings = settings_path
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();

    let window =
        AppWindow::new().inspect_err(|error| log!("startup: the window failed: {error}"))?;
    log!("startup: window built");
    let administrator = osflux_engine::is_elevated();
    log!("startup: running as Administrator: {administrator}");
    let shell = Shell::install(&window, settings, settings_path, administrator);
    log_first_frame(&window);
    window.show()?;
    log!("startup: window shown; starting the event loop");
    slint::Timer::single_shot(std::time::Duration::ZERO, || {
        log!("startup: event loop running");
    });
    shell.borrow().accept_dropped_files(&window);
    let result = slint::run_event_loop();
    log!("closing");
    result?;
    window.hide()
}

/// Logs when the first frame has been drawn, the moment Studio appears.
fn log_first_frame(window: &AppWindow) {
    let mut logged = false;
    let notifier = window.window().set_rendering_notifier(move |state, _| {
        if !logged && matches!(state, slint::RenderingState::AfterRendering) {
            logged = true;
            log!("startup: first frame shown");
        }
    });
    if let Err(error) = notifier {
        log!("startup: first frame time not available: {error}");
    }
}
