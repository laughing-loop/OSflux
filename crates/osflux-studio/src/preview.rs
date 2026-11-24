//! `--preview`, built only with the `preview` feature: shows a screen with
//! sample data, and with `--snapshot <file.png>` saves it as a PNG and exits.
//! Used for screenshots and design checks; it never touches a drive.

use std::path::PathBuf;
use std::time::Duration;

use slint::ComponentHandle;

use crate::AppWindow;

/// How long the window is given to lay out and draw before the snapshot.
const SNAPSHOT_DELAY: Duration = Duration::from_millis(500);

pub struct Options {
    snapshot: Option<PathBuf>,
}

impl Options {
    /// `Some` when the arguments ask for a preview.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut args = args.into_iter();
        if args.next().as_deref() != Some("--preview") {
            return None;
        }
        let mut snapshot = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--snapshot" => snapshot = args.next().map(PathBuf::from),
                other => eprintln!("--preview: ignoring unknown option {other}"),
            }
        }
        Some(Self { snapshot })
    }
}

pub fn run(options: Options) -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    let Some(path) = options.snapshot else {
        return window.run();
    };

    window.show()?;
    let weak = window.as_weak();
    slint::Timer::single_shot(SNAPSHOT_DELAY, move || {
        if let Some(window) = weak.upgrade() {
            if let Err(error) = save_snapshot(&window, &path) {
                eprintln!("--snapshot: {error}");
            }
        }
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop()
}

fn save_snapshot(window: &AppWindow, path: &PathBuf) -> Result<(), String> {
    let shot = window
        .window()
        .take_snapshot()
        .map_err(|error| format!("could not take a snapshot: {error}"))?;
    image::save_buffer(
        path,
        shot.as_bytes(),
        shot.width(),
        shot.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|error| format!("could not save {}: {error}", path.display()))?;
    eprintln!(
        "saved {} ({}x{})",
        path.display(),
        shot.width(),
        shot.height()
    );
    Ok(())
}
