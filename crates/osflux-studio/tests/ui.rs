//! Drives the real Studio window without a display, through Slint's testing
//! backend: what the shell shows on start, and what Settings changes.

mod support;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use osflux_studio::session::Jobs;
use osflux_studio::settings::{Settings, ThemeSetting};
use osflux_studio::shell::{EngineSetup, Shell};
use osflux_studio::{AppWindow, Screen, StepState, Studio, Theme, ThemeChoice};
use slint::{ComponentHandle, Model};
use support::{frame_05_drives, window, FakeEngine};

/// The shell with the sample engine, so nothing touches real drives.
fn install(
    window: &AppWindow,
    settings: Settings,
    settings_path: Option<PathBuf>,
    administrator: bool,
) -> Rc<RefCell<Shell>> {
    Shell::install_with(
        window,
        settings,
        settings_path,
        administrator,
        EngineSetup {
            engine: Arc::new(FakeEngine {
                drives: Mutex::new(frame_05_drives()),
            }),
            jobs: Jobs::Inline,
            clock: || "13:28".to_string(),
        },
    )
}

fn temp_settings_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "osflux-studio-ui-test-{}-{name}",
            std::process::id()
        ))
        .join("studio-settings.txt")
}

#[test]
fn starts_on_step_1_with_nothing_chosen() {
    let window = window();
    let _shell = install(&window, Settings::default(), None, true);
    let studio = window.global::<Studio>();

    assert_eq!(studio.get_screen(), Screen::Image);
    assert_eq!(studio.get_step_label(), "Step 1");
    assert_eq!(studio.get_title(), "Choose an image");
    assert!(!studio.get_can_go_back());
    assert!(!studio.get_can_go_next());

    let steps = studio.get_steps();
    let titles: Vec<String> = steps.iter().map(|step| step.title.to_string()).collect();
    assert_eq!(titles, ["Image", "Drive", "Safety check", "Confirm"]);
    assert_eq!(steps.row_data(0).unwrap().state, StepState::Current);
    assert!(steps
        .iter()
        .skip(1)
        .all(|step| step.state == StepState::Pending));

    // Drives are listed at once, so the Drive step is ready when reached.
    assert_eq!(studio.get_drives_scanned(), "13:28");
    assert!(studio.get_drives_listed());

    let summary = studio.get_summary();
    assert_eq!(summary.image, "Not chosen yet");
    assert_eq!(summary.drive, "Not chosen yet");
    assert!(!summary.show_windows_options);
}

#[test]
fn shows_whether_studio_runs_as_administrator() {
    let window = window();
    let _shell = install(&window, Settings::default(), None, false);
    assert!(!window.global::<Studio>().get_administrator());

    let window = self::window();
    let _shell = install(&window, Settings::default(), None, true);
    assert!(window.global::<Studio>().get_administrator());
}

#[test]
fn the_saved_theme_and_verification_are_applied_on_start() {
    let window = window();
    let settings = Settings {
        theme: ThemeSetting::Light,
        verify_after_writing: false,
    };
    let _shell = install(&window, settings, None, true);

    assert_eq!(window.global::<Theme>().get_choice(), ThemeChoice::Light);
    assert!(!window.global::<Theme>().get_dark());
    assert_eq!(
        window.global::<Studio>().get_theme_choice(),
        ThemeChoice::Light
    );
    assert!(!window.global::<Studio>().get_verify_after_writing());
}

#[test]
fn settings_change_the_theme_and_are_saved() {
    let path = temp_settings_path("theme");
    let window = window();
    let shell = install(&window, Settings::default(), Some(path.clone()), true);
    let studio = window.global::<Studio>();

    studio.invoke_set_theme(ThemeChoice::Dark);
    assert_eq!(window.global::<Theme>().get_choice(), ThemeChoice::Dark);
    assert!(window.global::<Theme>().get_dark());
    assert_eq!(studio.get_theme_choice(), ThemeChoice::Dark);
    assert_eq!(shell.borrow().settings().theme, ThemeSetting::Dark);
    assert_eq!(Settings::load(&path).theme, ThemeSetting::Dark);

    studio.invoke_set_theme(ThemeChoice::Light);
    assert!(!window.global::<Theme>().get_dark());
    assert_eq!(Settings::load(&path).theme, ThemeSetting::Light);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn turning_verification_off_is_saved_and_shown() {
    let path = temp_settings_path("verify");
    let window = window();
    let shell = install(&window, Settings::default(), Some(path.clone()), true);
    let studio = window.global::<Studio>();
    assert!(studio.get_verify_after_writing());

    studio.invoke_set_verify_after_writing(false);
    assert!(!studio.get_verify_after_writing());
    assert!(!shell.borrow().settings().verify_after_writing);
    assert!(!Settings::load(&path).verify_after_writing);

    studio.invoke_set_verify_after_writing(true);
    assert!(Settings::load(&path).verify_after_writing);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn about_carries_the_licence_texts_and_platform() {
    let window = window();
    let _shell = install(&window, Settings::default(), None, true);
    let studio = window.global::<Studio>();

    assert!(studio.get_gpl_text().contains("GNU GENERAL PUBLIC LICENSE"));
    assert!(studio
        .get_font_licence_text()
        .contains("SIL OPEN FONT LICENSE"));
    assert!(studio.get_platform().contains(std::env::consts::ARCH));
    assert_eq!(studio.get_version(), env!("CARGO_PKG_VERSION"));
}

#[test]
fn the_settings_and_about_buttons_open_their_dialogs() {
    let window = window();
    let _shell = install(&window, Settings::default(), None, true);
    window.show().expect("the window shows");
    let studio = window.global::<Studio>();

    let settings =
        i_slint_backend_testing::ElementHandle::find_by_accessible_label(&window, "Settings")
            .next()
            .expect("the Settings button");
    settings.invoke_accessible_default_action();
    assert!(studio.get_settings_open());

    let about = i_slint_backend_testing::ElementHandle::find_by_accessible_label(
        &window,
        "About OSflux Studio",
    )
    .next()
    .expect("the About button");
    about.invoke_accessible_default_action();
    assert!(studio.get_about_open());
}
