// A window, not a console, in release builds. Debug builds keep the console
// for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::time::Duration;

use osflux_studio::log;
use osflux_studio::platform;
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
    // Before any window exists: the renderer is chosen once per process.
    osflux_studio::renderer::select();

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
    slint::Timer::single_shot(Duration::ZERO, || {
        log!("startup: event loop running");
    });
    shell.borrow().accept_dropped_files(&window);
    let result = slint::run_event_loop();
    log!("closing");
    result?;
    window.hide()
}

/// Logs when the native window is ready and when the first frame has been
/// drawn, the moment Studio appears. The OpenGL renderer reports its frames;
/// the software renderer cannot, so on Windows the window itself is asked
/// whether it has been painted.
fn log_first_frame(window: &AppWindow) {
    let mut logged = false;
    let notifier = window.window().set_rendering_notifier(move |state, _| {
        if !logged && matches!(state, slint::RenderingState::AfterRendering) {
            logged = true;
            log!("startup: first frame shown");
        }
    });
    let reported = notifier.is_ok();
    if !reported {
        log!("startup: this renderer does not report frames; asking the window instead");
    }
    if cfg!(windows) {
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::ZERO, move || {
            watch_window(weak, reported, false, 0)
        });
    }
}

const WATCH_INTERVAL: Duration = Duration::from_millis(5);
const WATCH_ATTEMPTS: u32 = 2000;

/// Checks every 5 ms, for up to 10 seconds, until the native window exists
/// and then, unless the renderer reports frames, until it has been painted.
fn watch_window(window: slint::Weak<AppWindow>, reported: bool, ready: bool, attempt: u32) {
    let Some(app) = window.upgrade() else {
        return;
    };
    if attempt >= WATCH_ATTEMPTS {
        log!("startup: the window was not ready after 10 s");
        return;
    }
    let mut ready = ready;
    if let Some(handle) = platform::native_handle(app.window()) {
        if !ready {
            log!("startup: native window ready");
            ready = true;
            if reported {
                return;
            }
        }
        if platform::window_painted(handle) == Some(true) {
            log!("startup: first frame shown");
            return;
        }
    }
    slint::Timer::single_shot(WATCH_INTERVAL, move || {
        watch_window(window, reported, ready, attempt + 1)
    });
}
