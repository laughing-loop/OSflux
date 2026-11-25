//! `--preview`, built only with the `preview` feature: shows a view with
//! sample data and, with `--snapshot <file.png>`, saves it as a PNG and
//! exits. Used for screenshots and design checks; it never touches a drive
//! and never saves settings.
//!
//! `--preview <shell|settings|about|licences|gallery> [--theme dark|light]
//! [--not-admin] [--verify-off] [--snapshot file.png]`

use std::path::{Path, PathBuf};
use std::time::Duration;

use slint::ComponentHandle;

use crate::settings::{Settings, ThemeSetting};
use crate::shell::{self, Shell};
use crate::{AppWindow, Gallery, Studio, Theme};

/// How long a window is given to lay out and draw before the snapshot.
const SNAPSHOT_DELAY: Duration = Duration::from_millis(600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Shell,
    Settings,
    About,
    Licences,
    Gallery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub view: View,
    pub settings: Settings,
    pub administrator: bool,
    pub snapshot: Option<PathBuf>,
}

impl Options {
    /// `Some` when the arguments ask for a preview.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Option<Self> {
        let mut args = args.into_iter().peekable();
        if args.next().as_deref() != Some("--preview") {
            return None;
        }
        let mut options = Options {
            view: View::Shell,
            settings: Settings::default(),
            administrator: true,
            snapshot: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "shell" => options.view = View::Shell,
                "settings" => options.view = View::Settings,
                "about" => options.view = View::About,
                "licences" | "licenses" => options.view = View::Licences,
                "gallery" => options.view = View::Gallery,
                "--theme" => match args.next().as_deref() {
                    Some("dark") => options.settings.theme = ThemeSetting::Dark,
                    Some("light") => options.settings.theme = ThemeSetting::Light,
                    _ => options.settings.theme = ThemeSetting::System,
                },
                "--not-admin" => options.administrator = false,
                "--verify-off" => options.settings.verify_after_writing = false,
                "--snapshot" => options.snapshot = args.next().map(PathBuf::from),
                other => eprintln!("--preview: ignoring unknown option {other}"),
            }
        }
        Some(options)
    }
}

pub fn run(options: Options) -> Result<(), slint::PlatformError> {
    if options.view == View::Gallery {
        let gallery = Gallery::new()?;
        gallery
            .global::<Theme>()
            .set_choice(shell::theme_choice(options.settings.theme));
        return present(gallery, options.snapshot);
    }

    let window = AppWindow::new()?;
    let _shell = Shell::install(&window, options.settings, None, options.administrator);
    let studio = window.global::<Studio>();
    studio.set_drives_scanned("13:28".into());
    match options.view {
        View::Settings => studio.set_settings_open(true),
        View::About => studio.set_about_open(true),
        View::Licences => {
            studio.set_about_open(true);
            studio.set_showing_licences(true);
        }
        View::Shell | View::Gallery => {}
    }
    present(window, options.snapshot)
}

fn present<C: ComponentHandle + 'static>(
    component: C,
    snapshot: Option<PathBuf>,
) -> Result<(), slint::PlatformError> {
    let Some(path) = snapshot else {
        return component.run();
    };
    component.show()?;
    let weak = component.as_weak();
    slint::Timer::single_shot(SNAPSHOT_DELAY, move || {
        if let Some(component) = weak.upgrade() {
            if let Err(error) = save_snapshot(component.window(), &path) {
                eprintln!("--snapshot: {error}");
            }
        }
        let _ = slint::quit_event_loop();
    });
    slint::run_event_loop()
}

fn save_snapshot(window: &slint::Window, path: &Path) -> Result<(), String> {
    let shot = window
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

#[cfg(test)]
mod tests {
    use super::{Options, View};
    use crate::settings::ThemeSetting;

    fn parse(args: &[&str]) -> Option<Options> {
        Options::from_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn only_preview_arguments_start_a_preview() {
        assert!(parse(&[]).is_none());
        assert!(parse(&["--other"]).is_none());
        let options = parse(&["--preview"]).expect("a preview");
        assert_eq!(options.view, View::Shell);
        assert!(options.administrator);
    }

    #[test]
    fn reads_the_view_theme_and_states() {
        let options = parse(&[
            "--preview",
            "settings",
            "--theme",
            "light",
            "--not-admin",
            "--verify-off",
            "--snapshot",
            "out.png",
        ])
        .expect("a preview");
        assert_eq!(options.view, View::Settings);
        assert_eq!(options.settings.theme, ThemeSetting::Light);
        assert!(!options.settings.verify_after_writing);
        assert!(!options.administrator);
        assert_eq!(
            options.snapshot.as_deref(),
            Some(std::path::Path::new("out.png"))
        );
    }
}
