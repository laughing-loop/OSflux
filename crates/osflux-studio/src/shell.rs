//! Connects the window to Studio's state: settings, the header's privilege
//! status, the step rail, "This session", About, and the actions the shell
//! offers. The step screens build on this.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::about;
use crate::settings::{Settings, ThemeSetting};
use crate::steps::{self, Flow, StepId, StepState as RailState, StepView};
use crate::system;
use crate::{
    AppWindow, HintIcon, Screen, StepData, StepState, Studio, SummaryData, Theme, ThemeChoice,
};

/// Where Studio's own state lives while the window is open.
pub struct Shell {
    settings: Settings,
    /// Where settings are saved; `None` keeps them in memory (previews, tests).
    settings_path: Option<PathBuf>,
}

impl Shell {
    /// Fills the window and connects its actions. The returned handle keeps
    /// the state alive for as long as the window runs.
    pub fn install(
        window: &AppWindow,
        settings: Settings,
        settings_path: Option<PathBuf>,
        administrator: bool,
    ) -> Rc<RefCell<Shell>> {
        let shell = Rc::new(RefCell::new(Shell {
            settings,
            settings_path,
        }));

        let studio = window.global::<Studio>();
        studio.set_version(env!("CARGO_PKG_VERSION").into());
        studio.set_engine_version(env!("CARGO_PKG_VERSION").into());
        studio.set_administrator(administrator);
        studio.set_platform(about::platform().into());
        studio.set_gpl_text(about::GPL_TEXT.into());
        studio.set_font_licence_text(about::FONT_LICENCE_TEXT.into());

        apply_settings(window, settings);
        show_image_empty(window);
        connect(window, &shell);
        shell
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    fn update_settings(&mut self, settings: Settings) {
        self.settings = settings;
        if let Some(path) = &self.settings_path {
            if let Err(error) = settings.save(path) {
                eprintln!("could not save settings to {}: {error}", path.display());
            }
        }
    }
}

fn connect(window: &AppWindow, shell: &Rc<RefCell<Shell>>) {
    let studio = window.global::<Studio>();

    let weak = window.as_weak();
    let state = Rc::clone(shell);
    studio.on_set_theme(move |choice| {
        let Some(window) = weak.upgrade() else { return };
        let mut shell = state.borrow_mut();
        let settings = Settings {
            theme: theme_setting(choice),
            ..shell.settings
        };
        shell.update_settings(settings);
        apply_settings(&window, settings);
    });

    let weak = window.as_weak();
    let state = Rc::clone(shell);
    studio.on_set_verify_after_writing(move |on| {
        let Some(window) = weak.upgrade() else { return };
        let mut shell = state.borrow_mut();
        let settings = Settings {
            verify_after_writing: on,
            ..shell.settings
        };
        shell.update_settings(settings);
        apply_settings(&window, settings);
    });

    studio.on_open_audit_log(|| match osflux_engine::AuditLog::default_location() {
        Some(log) => {
            if let Err(error) = system::show_file(log.path()) {
                eprintln!(
                    "could not open the audit log {}: {error}",
                    log.path().display()
                );
            }
        }
        None => eprintln!("OSflux could not find where its audit log is kept"),
    });

    studio.on_open_link(|id| {
        if let Some(url) = about::link(id.as_str()) {
            if let Err(error) = system::open_url(url) {
                eprintln!("could not open {url}: {error}");
            }
        }
    });
}

/// Shows the theme and verification settings everywhere they appear.
pub fn apply_settings(window: &AppWindow, settings: Settings) {
    let choice = theme_choice(settings.theme);
    window.global::<Theme>().set_choice(choice);
    let studio = window.global::<Studio>();
    studio.set_theme_choice(choice);
    studio.set_verify_after_writing(settings.verify_after_writing);
}

/// Step 1 before an image is chosen (frame 01).
pub fn show_image_empty(window: &AppWindow) {
    let studio = window.global::<Studio>();
    studio.set_screen(Screen::Image);
    studio.set_steps(rail_model(steps::build_rail(
        Flow::Undecided,
        StepId::Image,
        None,
        &[],
    )));
    studio.set_step_label(steps::step_label(Flow::Undecided, StepId::Image).into());
    studio.set_title("Choose an image".into());
    studio.set_description(
        "Pick the operating system image to put on a USB stick or SD card. OSflux reads it and explains what it found."
            .into(),
    );
    studio.set_hint("Choose an image to continue.".into());
    studio.set_hint_icon(HintIcon::None);
    studio.set_can_go_back(false);
    studio.set_can_go_next(false);
    studio.set_summary(SummaryData {
        image: "Not chosen yet".into(),
        image_full: SharedString::new(),
        image_chosen: false,
        drive: "Not chosen yet".into(),
        show_windows_options: false,
        windows_options: SharedString::new(),
    });
}

pub fn rail_model(steps: Vec<StepView>) -> ModelRc<StepData> {
    let items: Vec<StepData> = steps
        .into_iter()
        .map(|step| StepData {
            title: step.title.into(),
            subtitle: step.subtitle.into(),
            subtitle_mono: step.subtitle_mono,
            subtitle_full: step.subtitle_full.into(),
            state: match step.state {
                RailState::Pending => StepState::Pending,
                RailState::Current => StepState::Current,
                RailState::Done => StepState::Done,
                RailState::Blocked => StepState::Blocked,
            },
            clickable: step.clickable,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

pub fn theme_choice(setting: ThemeSetting) -> ThemeChoice {
    match setting {
        ThemeSetting::System => ThemeChoice::System,
        ThemeSetting::Dark => ThemeChoice::Dark,
        ThemeSetting::Light => ThemeChoice::Light,
    }
}

pub fn theme_setting(choice: ThemeChoice) -> ThemeSetting {
    match choice {
        ThemeChoice::System => ThemeSetting::System,
        ThemeChoice::Dark => ThemeSetting::Dark,
        ThemeChoice::Light => ThemeSetting::Light,
    }
}
