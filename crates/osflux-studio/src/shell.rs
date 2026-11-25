//! Connects the window to Studio's state: settings, the header's privilege
//! status, About, and the step actions, which the session carries out.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::about;
use crate::platform;
use crate::safety_step::Fix;
use crate::session::{self, Engine, Jobs, LiveEngine, Session};
use crate::settings::{Settings, ThemeSetting};
use crate::steps::{StepState as RailState, StepView};
use crate::system;
use crate::{AppWindow, StepData, StepState, Studio, Theme, ThemeChoice};

/// Where Studio's own state lives while the window is open.
pub struct Shell {
    settings: Settings,
    /// Where settings are saved; `None` keeps them in memory (previews, tests).
    settings_path: Option<PathBuf>,
    session: Rc<RefCell<Session>>,
}

/// How a shell talks to the engine: the real one on worker threads, or a
/// stand-in straight away for tests and previews.
pub struct EngineSetup {
    pub engine: Arc<dyn Engine>,
    pub jobs: Jobs,
    /// The local time, as `HH:MM`, for "Drives scanned".
    pub clock: fn() -> String,
}

impl EngineSetup {
    pub fn live() -> Self {
        Self {
            engine: Arc::new(LiveEngine),
            jobs: Jobs::Background,
            clock: platform::clock::local_time_of_day,
        }
    }
}

impl Shell {
    /// Fills the window, connects its actions to the real engine and starts
    /// looking for drives. The returned handle keeps the state alive for as
    /// long as the window runs.
    pub fn install(
        window: &AppWindow,
        settings: Settings,
        settings_path: Option<PathBuf>,
        administrator: bool,
    ) -> Rc<RefCell<Shell>> {
        Self::install_with(
            window,
            settings,
            settings_path,
            administrator,
            EngineSetup::live(),
        )
    }

    pub fn install_with(
        window: &AppWindow,
        settings: Settings,
        settings_path: Option<PathBuf>,
        administrator: bool,
        setup: EngineSetup,
    ) -> Rc<RefCell<Shell>> {
        let session = Session::new(
            window,
            setup.engine,
            setup.jobs,
            setup.clock,
            settings.verify_after_writing,
        );
        let shell = Rc::new(RefCell::new(Shell {
            settings,
            settings_path,
            session: Rc::clone(&session),
        }));

        let studio = window.global::<Studio>();
        studio.set_version(env!("CARGO_PKG_VERSION").into());
        studio.set_engine_version(env!("CARGO_PKG_VERSION").into());
        studio.set_administrator(administrator);
        studio.set_platform(about::platform().into());
        studio.set_gpl_text(about::GPL_TEXT.into());
        studio.set_font_licence_text(about::FONT_LICENCE_TEXT.into());

        apply_settings(window, settings);
        connect(window, &shell, &session);
        session.borrow_mut().start();
        shell
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    pub fn session(&self) -> Rc<RefCell<Session>> {
        Rc::clone(&self.session)
    }

    /// Lets image files be dropped onto the window. Call before the event
    /// loop runs: the native window only exists once it does, so the setup
    /// waits for it from inside the loop, checking every 50 ms for up to
    /// 10 seconds. Each step is written to Studio's log.
    pub fn accept_dropped_files(&self, window: &AppWindow) {
        if !cfg!(windows) {
            crate::log!("drop: not supported on this platform; Browse works");
            return;
        }
        let window = window.as_weak();
        let session = Rc::downgrade(&self.session);
        slint::Timer::single_shot(Duration::ZERO, move || {
            set_up_drop(window, session, 0);
        });
    }

    fn update_settings(&mut self, settings: Settings) {
        self.settings = settings;
        if let Some(path) = &self.settings_path {
            if let Err(error) = settings.save(path) {
                crate::log!("could not save settings to {}: {error}", path.display());
            }
        }
    }
}

const DROP_SETUP_ATTEMPTS: u32 = 200;
const DROP_SETUP_INTERVAL: Duration = Duration::from_millis(50);

fn set_up_drop(window: slint::Weak<AppWindow>, session: Weak<RefCell<Session>>, attempt: u32) {
    let Some(app) = window.upgrade() else {
        return;
    };
    let Some(handle) = platform::native_handle(app.window()) else {
        if attempt == 0 {
            crate::log!("drop: waiting for the native window");
        }
        if attempt + 1 >= DROP_SETUP_ATTEMPTS {
            crate::log!(
                "drop: no native window after {} ms; dropping files will not work",
                DROP_SETUP_ATTEMPTS as u64 * DROP_SETUP_INTERVAL.as_millis() as u64
            );
            return;
        }
        slint::Timer::single_shot(DROP_SETUP_INTERVAL, move || {
            set_up_drop(window, session, attempt + 1);
        });
        return;
    };
    crate::log!("drop: native window handle {handle:#x} found after {attempt} retries");
    let handler = Box::new(move |path: PathBuf| {
        session::open_dropped_image(session.clone(), path);
    });
    if let Err(error) = platform::drop::accept_dropped_files(handle, handler) {
        crate::log!("drop: dropping files onto the window will not work: {error}");
    }
}

fn connect(window: &AppWindow, shell: &Rc<RefCell<Shell>>, session: &Rc<RefCell<Session>>) {
    let studio = window.global::<Studio>();

    let weak = window.as_weak();
    let state = Rc::clone(session);
    studio.on_choose_image(move || {
        if let Some(window) = weak.upgrade() {
            session::choose_image(&window, &state);
        }
    });

    let state = Rc::clone(session);
    studio.on_expected_sha_edited(move |text| state.borrow_mut().expected_sha_edited(&text));

    let state = Rc::clone(session);
    studio.on_next(move || state.borrow_mut().next());

    let state = Rc::clone(session);
    studio.on_back(move || state.borrow_mut().back());

    let state = Rc::clone(session);
    studio.on_go_to_step(move |index| {
        if let Ok(index) = usize::try_from(index) {
            state.borrow_mut().go_to_step(index);
        }
    });

    let state = Rc::clone(session);
    studio.on_select_drive(move |position| {
        if let Ok(position) = usize::try_from(position) {
            state.borrow_mut().select_drive(position);
        }
    });

    let state = Rc::clone(session);
    studio.on_refresh(move || state.borrow_mut().refresh());

    let state = Rc::clone(session);
    studio.on_set_option(move |index, on| {
        if let Ok(index) = usize::try_from(index) {
            state.borrow_mut().set_option(index, on);
        }
    });

    let state = Rc::clone(session);
    studio.on_account_name_edited(move |text| state.borrow_mut().account_name_edited(&text));

    let state = Rc::clone(session);
    studio.on_typed_edited(move |text| state.borrow_mut().typed_edited(&text));

    let state = Rc::clone(session);
    studio.on_confirm_again(move || state.borrow_mut().confirm_again());

    let state = Rc::clone(session);
    studio.on_write(move || state.borrow_mut().write());

    let weak = window.as_weak();
    let state = Rc::clone(session);
    studio.on_fix(move || {
        let fix = state.borrow_mut().fix();
        if fix == Some(Fix::ChooseImage) {
            if let Some(window) = weak.upgrade() {
                session::choose_image(&window, &state);
            }
        }
    });

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
        shell.session.borrow_mut().set_verify(on);
    });

    studio.on_open_audit_log(|| match osflux_engine::AuditLog::default_location() {
        Some(log) => {
            if let Err(error) = system::show_file(log.path()) {
                crate::log!(
                    "could not open the audit log {}: {error}",
                    log.path().display()
                );
            }
        }
        None => crate::log!("OSflux could not find where its audit log is kept"),
    });

    studio.on_open_link(|id| {
        if let Some(url) = about::link(id.as_str()) {
            if let Err(error) = system::open_url(url) {
                crate::log!("could not open {url}: {error}");
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
