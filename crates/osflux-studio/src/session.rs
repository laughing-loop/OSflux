//! One pass through the steps: the chosen image, the drives, the chosen
//! drive and the safety check, and what the window shows at each step.
//!
//! Reading an image, listing drives and running the safety check can take
//! seconds, so they run on a worker thread and their results come back to
//! the UI thread. Every verdict comes from the engine; the session only
//! keeps what the person chose and shows what the engine said about it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use osflux_engine::{
    assess_all_targets, run_safety_check, SafetyCheck, SafetyCheckError, Sha256Hex,
    Sha256ParseError, TargetAssessment, TargetAssessmentError,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::drive_step::{drive_hint, ChosenDrive, DriveList};
use crate::format::{middle_truncate, size_short};
use crate::image_step::{analyze_image, image_view, read_error_text, ImageAnalysis, ImageView};
use crate::safety_step::{
    failure_view, safety_view, Context, Detail, Fix, SafetyFailure, SafetyView,
};
use crate::shell::rail_model;
use crate::steps::{
    build_rail, step_label, steps_for, Flow, StepId, StepView, Subtitle, RAIL_NAME_CHARS,
};
use crate::{
    AppWindow, BlockedDriveData, CheckData, DetailRowData, DriveData, DriveKind, HintIcon,
    NoteData, PlanRowData, Screen, Status, Studio, SummaryData,
};

/// The engine calls a session makes. [`LiveEngine`] is the real engine;
/// previews and tests give sample drives instead.
pub trait Engine: Send + Sync {
    fn analyze_image(&self, path: &Path) -> Result<ImageAnalysis, String>;
    fn scan_drives(&self) -> Vec<TargetAssessment>;
    fn safety_check(&self, image: &Path, drive: &ChosenDrive)
        -> Result<SafetyCheck, SafetyFailure>;
}

pub struct LiveEngine;

impl Engine for LiveEngine {
    fn analyze_image(&self, path: &Path) -> Result<ImageAnalysis, String> {
        analyze_image(path)
    }

    fn scan_drives(&self) -> Vec<TargetAssessment> {
        assess_all_targets()
    }

    fn safety_check(
        &self,
        image: &Path,
        drive: &ChosenDrive,
    ) -> Result<SafetyCheck, SafetyFailure> {
        run_safety_check(image, drive.index, drive.identity.as_deref()).map_err(|error| match error
        {
            SafetyCheckError::Target(TargetAssessmentError::NotFound { .. }) => {
                SafetyFailure::DriveGone
            }
            SafetyCheckError::Image(error) => SafetyFailure::Image(error.to_string()),
        })
    }
}

/// Where engine calls run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jobs {
    /// On a worker thread, with the result handed back to the UI thread.
    Background,
    /// Straight away, on the calling thread: for tests and previews.
    Inline,
}

thread_local! {
    // The session that background results are delivered to, on the UI thread.
    static ACTIVE: RefCell<Weak<RefCell<Session>>> = const { RefCell::new(Weak::new()) };
}

struct LoadedImage {
    path: PathBuf,
    analysis: ImageAnalysis,
    view: ImageView,
}

pub struct Session {
    window: slint::Weak<AppWindow>,
    engine: Arc<dyn Engine>,
    jobs: Jobs,
    clock: fn() -> String,
    verify: bool,
    step: StepId,

    image: Option<LoadedImage>,
    image_busy: bool,
    image_error: String,
    image_job: u64,
    expected_sha: Option<Sha256Hex>,
    sha_error: String,

    drives: Option<DriveList>,
    drives_busy: bool,
    drives_scanned: String,
    drives_job: u64,
    blocked_open_set: bool,
    chosen: Option<ChosenDrive>,

    safety: Option<Result<SafetyCheck, SafetyFailure>>,
    safety_busy: bool,
    safety_job: u64,
}

impl Session {
    pub fn new(
        window: &AppWindow,
        engine: Arc<dyn Engine>,
        jobs: Jobs,
        clock: fn() -> String,
        verify: bool,
    ) -> Rc<RefCell<Session>> {
        let session = Rc::new(RefCell::new(Session {
            window: window.as_weak(),
            engine,
            jobs,
            clock,
            verify,
            step: StepId::Image,
            image: None,
            image_busy: false,
            image_error: String::new(),
            image_job: 0,
            expected_sha: None,
            sha_error: String::new(),
            drives: None,
            drives_busy: false,
            drives_scanned: String::new(),
            drives_job: 0,
            blocked_open_set: false,
            chosen: None,
            safety: None,
            safety_busy: false,
            safety_job: 0,
        }));
        if jobs == Jobs::Background {
            ACTIVE.with(|active| *active.borrow_mut() = Rc::downgrade(&session));
        }
        session
    }

    /// Shows step 1 and starts looking for drives.
    pub fn start(&mut self) {
        self.render();
        self.scan_drives();
    }

    pub fn step(&self) -> StepId {
        self.step
    }

    pub fn chosen_drive(&self) -> Option<&ChosenDrive> {
        self.chosen.as_ref()
    }

    /// The expected SHA-256 the person entered, once it is a valid one.
    pub fn expected_sha(&self) -> Option<&Sha256Hex> {
        self.expected_sha.as_ref()
    }

    pub fn set_verify(&mut self, verify: bool) {
        self.verify = verify;
        self.render();
    }

    fn flow(&self) -> Flow {
        self.image
            .as_ref()
            .map(|image| image.analysis.flow())
            .unwrap_or(Flow::Undecided)
    }

    // ---- Actions ------------------------------------------------------

    /// Reads a newly chosen or dropped image and shows step 1 for it.
    pub fn open_image(&mut self, path: PathBuf) {
        self.image_job += 1;
        let job = self.image_job;
        self.image_busy = true;
        self.image_error.clear();
        self.step = StepId::Image;
        self.safety = None;
        self.render();
        self.run(
            move |engine| {
                let result = engine.analyze_image(&path);
                (path, result)
            },
            Session::image_read,
            job,
        );
    }

    fn image_chosen(&mut self, result: Result<Option<PathBuf>, String>) {
        match result {
            Ok(Some(path)) => self.open_image(path),
            Ok(None) => {}
            Err(error) => {
                self.image_error = format!("OSflux couldn't open the file chooser: {error}.");
                self.render();
            }
        }
    }

    pub fn next(&mut self) {
        match self.step {
            StepId::Image => {
                let ready = !self.image_busy
                    && self
                        .image
                        .as_ref()
                        .is_some_and(|image| image.analysis.can_continue());
                if ready {
                    self.go_to(StepId::Drive);
                    return;
                }
            }
            StepId::Drive => {
                if self.chosen_is_listed() {
                    self.step = StepId::Safety;
                    self.run_safety_check();
                    return;
                }
            }
            StepId::Safety => {
                if self.safety_view().is_some_and(|view| view.can_continue) {
                    self.step = if self.flow() == Flow::Windows {
                        StepId::WindowsOptions
                    } else {
                        StepId::Confirm
                    };
                }
            }
            StepId::WindowsOptions => self.step = StepId::Confirm,
            StepId::Confirm => {}
        }
        self.render();
    }

    pub fn back(&mut self) {
        let steps = steps_for(self.flow());
        if let Some(position) = steps.iter().position(|step| *step == self.step) {
            if position > 0 {
                self.go_to(steps[position - 1]);
                return;
            }
        }
        self.render();
    }

    /// A done step in the rail was clicked.
    pub fn go_to_step(&mut self, index: usize) {
        let steps = steps_for(self.flow());
        let current = steps
            .iter()
            .position(|step| *step == self.step)
            .unwrap_or(0);
        if index < current {
            self.go_to(steps[index]);
        }
    }

    fn go_to(&mut self, step: StepId) {
        self.step = step;
        if step == StepId::Drive && !self.drives_busy {
            // Sticks come and go; list them afresh each time.
            self.scan_drives();
        }
        self.render();
    }

    pub fn select_drive(&mut self, position: usize) {
        if let Some(drive) = self
            .drives
            .as_ref()
            .and_then(|list| list.allowed.get(position))
        {
            self.chosen = Some(drive.chosen());
            self.safety = None;
        }
        self.render();
    }

    pub fn refresh(&mut self) {
        self.scan_drives();
    }

    /// The button next to a blocked verdict. Returns `Some(ChooseImage)`
    /// when the caller should open the Open dialog.
    pub fn fix(&mut self) -> Option<Fix> {
        match self.safety_view().and_then(|view| view.fix) {
            Some(Fix::ChooseDrive) => {
                self.go_to(StepId::Drive);
                None
            }
            other => other,
        }
    }

    /// The expected SHA-256 was edited. Letters are shown in upper case;
    /// the value counts only once it is a whole, valid SHA-256.
    pub fn expected_sha_edited(&mut self, text: &str) {
        let upper = text.to_ascii_uppercase();
        if upper != text {
            if let Some(window) = self.window.upgrade() {
                window
                    .global::<Studio>()
                    .set_expected_sha(upper.clone().into());
            }
        }
        if upper.trim().is_empty() {
            self.expected_sha = None;
            self.sha_error.clear();
        } else {
            match Sha256Hex::parse(&upper) {
                Ok(hash) => {
                    self.expected_sha = Some(hash);
                    self.sha_error.clear();
                }
                Err(error) => {
                    self.expected_sha = None;
                    self.sha_error = sha_error_text(error);
                }
            }
        }
        if let Some(window) = self.window.upgrade() {
            window
                .global::<Studio>()
                .set_sha_error(self.sha_error.clone().into());
        }
    }

    // ---- Engine calls and their results -------------------------------

    fn scan_drives(&mut self) {
        self.drives_job += 1;
        let job = self.drives_job;
        self.drives_busy = true;
        self.render();
        self.run(|engine| engine.scan_drives(), Session::drives_listed, job);
    }

    fn run_safety_check(&mut self) {
        let (Some(image), Some(chosen)) = (&self.image, &self.chosen) else {
            self.render();
            return;
        };
        let path = image.path.clone();
        let chosen = chosen.clone();
        self.safety_job += 1;
        let job = self.safety_job;
        self.safety_busy = true;
        self.safety = None;
        self.render();
        self.run(
            move |engine| engine.safety_check(&path, &chosen),
            Session::safety_checked,
            job,
        );
    }

    fn image_read(&mut self, job: u64, (path, result): (PathBuf, Result<ImageAnalysis, String>)) {
        if job != self.image_job {
            return;
        }
        self.image_busy = false;
        match result {
            Ok(analysis) => {
                let view = image_view(&analysis);
                self.image = Some(LoadedImage {
                    path,
                    analysis,
                    view,
                });
                self.expected_sha = None;
                self.sha_error.clear();
                if let Some(window) = self.window.upgrade() {
                    let studio = window.global::<Studio>();
                    studio.set_expected_sha(SharedString::new());
                    studio.set_sha_error(SharedString::new());
                }
            }
            Err(error) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                self.image = None;
                self.image_error = read_error_text(&name, &error);
            }
        }
        self.render();
    }

    fn drives_listed(&mut self, job: u64, assessments: Vec<TargetAssessment>) {
        if job != self.drives_job {
            return;
        }
        self.drives_busy = false;
        self.drives_scanned = (self.clock)();
        let list = DriveList::from_assessments(&assessments);
        if !self.blocked_open_set {
            // Frame 05 opens "Not allowed"; frame 06, with nothing to pick,
            // keeps it closed. After that it stays as the person left it.
            if let Some(window) = self.window.upgrade() {
                window
                    .global::<Studio>()
                    .set_blocked_open(!list.allowed.is_empty());
            }
            self.blocked_open_set = true;
        }
        let still_there = self
            .chosen
            .as_ref()
            .is_some_and(|chosen| list.position_of(chosen).is_some());
        self.drives = Some(list);
        if self.chosen.is_some() && !still_there {
            match self.step {
                StepId::Drive => {
                    self.chosen = None;
                    self.safety = None;
                }
                // The engine looks again, and says what changed.
                StepId::Safety => {
                    self.run_safety_check();
                    return;
                }
                _ => {}
            }
        }
        self.render();
    }

    fn safety_checked(&mut self, job: u64, result: Result<SafetyCheck, SafetyFailure>) {
        if job != self.safety_job {
            return;
        }
        self.safety_busy = false;
        self.safety = Some(result);
        if let Some(window) = self.window.upgrade() {
            // Frame 07 shows the details; frame 08, blocked, folds them away.
            let blocked = self.safety_view().is_none_or(|view| view.rail_blocked);
            window.global::<Studio>().set_details_open(!blocked);
        }
        self.render();
    }

    fn run<T: Send + 'static>(
        &mut self,
        work: impl FnOnce(&dyn Engine) -> T + Send + 'static,
        apply: fn(&mut Session, u64, T),
        job: u64,
    ) {
        match self.jobs {
            Jobs::Inline => {
                let result = work(&*self.engine);
                apply(self, job, result);
            }
            Jobs::Background => {
                let engine = Arc::clone(&self.engine);
                std::thread::spawn(move || {
                    let result = work(&*engine);
                    let _ = slint::invoke_from_event_loop(move || {
                        apply_when_free(move |session| apply(session, job, result));
                    });
                });
            }
        }
    }

    // ---- What the window shows ----------------------------------------

    fn safety_view(&self) -> Option<SafetyView> {
        if self.safety_busy {
            return None;
        }
        match self.safety.as_ref()? {
            Ok(check) => {
                let image = self.image.as_ref()?;
                let drive_kind = self
                    .drives
                    .as_ref()
                    .zip(self.chosen.as_ref())
                    .and_then(|(list, chosen)| {
                        list.position_of(chosen).map(|i| list.allowed[i].kind)
                    })
                    .unwrap_or(DriveKind::Usb);
                Some(safety_view(
                    check,
                    Context {
                        flow: self.flow(),
                        verify: self.verify,
                        family: image.analysis.family,
                        filesystem_strategy: image.analysis.filesystem_strategy,
                        drive_kind,
                    },
                ))
            }
            Err(failure) => Some(failure_view(failure)),
        }
    }

    fn chosen_is_listed(&self) -> bool {
        self.drives
            .as_ref()
            .zip(self.chosen.as_ref())
            .is_some_and(|(list, chosen)| list.position_of(chosen).is_some())
    }

    pub fn render(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let studio = window.global::<Studio>();
        let flow = self.flow();
        let safety = self.safety_view();

        studio.set_steps(rail_model(self.rail(flow, safety.as_ref())));
        studio.set_summary(self.summary(flow));
        studio.set_drives_busy(self.drives_busy);
        studio.set_drives_listed(self.drives.is_some());
        studio.set_drives_scanned(self.drives_scanned.clone().into());
        studio.set_offer_another_image(false);
        studio.set_hint_icon(HintIcon::None);

        let mut label = step_label(flow, self.step);
        match self.step {
            StepId::Image => self.render_image(&studio),
            StepId::Drive => self.render_drives(&studio),
            StepId::Safety => {
                label.push_str(" · Safety check verdict");
                self.render_safety(&studio, safety.as_ref());
            }
            StepId::WindowsOptions | StepId::Confirm => self.render_coming_next(&studio),
        }
        studio.set_step_label(label.into());
    }

    fn rail(&self, flow: Flow, safety: Option<&SafetyView>) -> Vec<StepView> {
        let mut subtitles = Vec::new();
        if let Some(image) = &self.image {
            subtitles.push((
                StepId::Image,
                Subtitle::FileName(image.analysis.plan.file_name.clone()),
            ));
        }
        if self.step == StepId::Drive {
            let none = self
                .drives
                .as_ref()
                .is_some_and(|list| list.allowed.is_empty());
            let text = if none {
                "No drive yet"
            } else {
                "Choose one drive"
            };
            subtitles.push((StepId::Drive, Subtitle::Text(text.to_string())));
        } else if let Some(chosen) = &self.chosen {
            subtitles.push((StepId::Drive, Subtitle::Text(chosen.model.clone())));
        }
        let steps = steps_for(flow);
        let reached = |step: StepId| {
            let at = |id| steps.iter().position(|s| *s == id);
            at(self.step) >= at(step)
        };
        if reached(StepId::Safety) {
            if self.safety_busy {
                subtitles.push((StepId::Safety, Subtitle::Text("Checking…".to_string())));
            } else if let Some(view) = safety {
                subtitles.push((StepId::Safety, Subtitle::Text(view.rail_subtitle.clone())));
            }
        }
        let blocked = (self.step == StepId::Safety && safety.is_some_and(|view| view.rail_blocked))
            .then_some(StepId::Safety);
        build_rail(flow, self.step, blocked, &subtitles)
    }

    fn summary(&self, flow: Flow) -> SummaryData {
        let (image, image_full) = match &self.image {
            Some(image) => {
                let name = &image.analysis.plan.file_name;
                let shown = middle_truncate(name, RAIL_NAME_CHARS);
                let full = if shown == *name {
                    String::new()
                } else {
                    name.clone()
                };
                (shown, full)
            }
            None => ("Not chosen yet".to_string(), String::new()),
        };
        SummaryData {
            image: image.into(),
            image_full: image_full.into(),
            image_chosen: self.image.is_some(),
            drive: self
                .chosen
                .as_ref()
                .map(|chosen| chosen.model.clone())
                .unwrap_or_else(|| "Not chosen yet".to_string())
                .into(),
            show_windows_options: flow == Flow::Windows,
            windows_options: "None".into(),
        }
    }

    fn render_image(&self, studio: &Studio<'_>) {
        studio.set_screen(Screen::Image);
        studio.set_image_busy(self.image_busy);
        studio.set_image_error(self.image_error.clone().into());
        studio.set_can_go_back(false);
        match (&self.image, self.image_busy) {
            (Some(image), false) => {
                let view = &image.view;
                studio.set_image_chosen(true);
                studio.set_image_name(view.file_name.clone().into());
                studio.set_image_path(view.path.clone().into());
                studio.set_image_size(view.size.clone().into());
                studio.set_plan_status(view.status);
                studio.set_plan_status_label(view.status_label.into());
                studio.set_plan_rows(model(view.rows.iter().map(|row| PlanRowData {
                    key: row.key.into(),
                    value: row.value.clone().into(),
                    mono: row.mono,
                    value_mark: row.value_mark,
                    detail: row.detail.clone().into(),
                    detail_mark: row.detail_mark,
                    secondary: row.secondary,
                })));
                studio.set_plan_notes(model(view.notes.iter().map(|note| NoteData {
                    status: note.status,
                    text: note.text.clone().into(),
                })));
                studio.set_title(view.title.into());
                studio.set_description(view.description.clone().into());
                studio.set_hint(view.hint.into());
                studio.set_hint_icon(view.hint_icon);
                let can_continue = image.analysis.can_continue();
                studio.set_can_go_next(can_continue);
                studio.set_offer_another_image(!can_continue);
            }
            _ => {
                studio.set_image_chosen(false);
                studio.set_title("Choose an image".into());
                studio.set_description(
                    "Pick the operating system image to put on a USB stick or SD card. OSflux reads it and explains what it found."
                        .into(),
                );
                studio.set_hint(
                    if self.image_busy {
                        "Reading the image…"
                    } else {
                        "Choose an image to continue."
                    }
                    .into(),
                );
                studio.set_can_go_next(false);
            }
        }
    }

    fn render_drives(&self, studio: &Studio<'_>) {
        studio.set_screen(Screen::Drive);
        studio.set_title("Choose a drive".into());
        studio.set_description(
            "Only drives the engine allows can be picked. Everything on the drive you pick will be erased."
                .into(),
        );
        studio.set_image_needs(
            self.image
                .as_ref()
                .map(|image| size_short(image.analysis.plan.size_bytes))
                .unwrap_or_default()
                .into(),
        );
        let empty = DriveList::default();
        let list = self.drives.as_ref().unwrap_or(&empty);
        let selected = self
            .chosen
            .as_ref()
            .and_then(|chosen| list.position_of(chosen));
        studio.set_drives(model(list.allowed.iter().enumerate().map(
            |(position, drive)| DriveData {
                model: drive.model.clone().into(),
                path: drive.path.clone().into(),
                size: drive.size.clone().into(),
                bus: drive.bus.clone().into(),
                media: drive.media.clone().into(),
                kind: drive.kind,
                status: drive.status,
                status_label: drive.status_label.into(),
                status_detail: drive.status_detail.into(),
                selected: selected == Some(position),
            },
        )));
        studio.set_blocked_drives(model(list.blocked.iter().map(|drive| BlockedDriveData {
            model: drive.model.clone().into(),
            path: drive.path.clone().into(),
            size: drive.size.clone().into(),
            bus: drive.bus.clone().into(),
            reason: drive.reason.clone().into(),
            kind: drive.kind,
        })));
        let chosen = selected.and(self.chosen.as_ref());
        studio.set_hint(drive_hint(self.drives.as_ref(), chosen).into());
        studio.set_can_go_back(true);
        studio.set_can_go_next(selected.is_some());
    }

    fn render_safety(&self, studio: &Studio<'_>, view: Option<&SafetyView>) {
        studio.set_screen(Screen::Safety);
        studio.set_safety_busy(self.safety_busy);
        studio.set_can_go_back(true);
        let Some(view) = view else {
            studio.set_title("Checking…".into());
            studio.set_description(
                "The engine is reading the image and the drive again before anything is written."
                    .into(),
            );
            studio.set_verdict(Status::None);
            studio.set_checks(model(std::iter::empty()));
            studio.set_details(model(std::iter::empty()));
            studio.set_checks_passed(SharedString::new());
            studio.set_fix_label(SharedString::new());
            studio.set_hint("Running the safety check…".into());
            studio.set_can_go_next(false);
            return;
        };
        studio.set_title(view.title.into());
        studio.set_description(view.description.clone().into());
        studio.set_verdict(view.status);
        studio.set_checks_passed(view.count.clone().into());
        studio.set_fix_label(view.fix.map(Fix::label).unwrap_or_default().into());
        studio.set_checks(model(view.rows.iter().map(|row| CheckData {
            status: row.status,
            title: row.title.into(),
            detail: row.detail.clone().into(),
            value: row.value.clone().into(),
        })));
        studio.set_details(details(&view.details_left, &view.details_right));
        studio.set_hint(view.hint.into());
        studio.set_can_go_next(view.can_continue);
    }

    /// Windows options and Confirm arrive in the next pull request.
    fn render_coming_next(&self, studio: &Studio<'_>) {
        let windows_options = self.step == StepId::WindowsOptions;
        studio.set_screen(if windows_options {
            Screen::WindowsOptions
        } else {
            Screen::Confirm
        });
        studio.set_title(
            if windows_options {
                "Windows options"
            } else {
                "Confirm the drive"
            }
            .into(),
        );
        studio.set_description(
            "This step comes in the next Studio update. Nothing has been written to the drive."
                .into(),
        );
        studio.set_hint("Coming in the next update.".into());
        studio.set_can_go_back(true);
        studio.set_can_go_next(windows_options);
    }
}

/// Browse… or Change image: asks for a file and reads it. On Windows the
/// Open dialog is modal to the window and runs here, with the session not
/// borrowed while it is open; elsewhere the chooser runs on a worker thread.
pub fn choose_image(window: &AppWindow, session: &Rc<RefCell<Session>>) {
    if cfg!(windows) {
        let owner = crate::platform::native_handle(window.window());
        let result = crate::platform::picker::choose_image(owner);
        session.borrow_mut().image_chosen(result);
    } else {
        let _ = window;
        if session.borrow().jobs == Jobs::Inline {
            let result = crate::platform::picker::choose_image(None);
            session.borrow_mut().image_chosen(result);
            return;
        }
        std::thread::spawn(move || {
            let result = crate::platform::picker::choose_image(None);
            let _ = slint::invoke_from_event_loop(move || {
                apply_when_free(move |session| session.image_chosen(result));
            });
        });
    }
}

/// Reads a file dropped onto the window, once the event that delivered it
/// has finished.
pub fn open_dropped_image(session: Weak<RefCell<Session>>, path: PathBuf) {
    slint::Timer::single_shot(Duration::ZERO, move || {
        let Some(session) = session.upgrade() else {
            return;
        };
        match session.try_borrow_mut() {
            Ok(mut session) => session.open_image(path),
            Err(_) => open_dropped_image(Rc::downgrade(&session), path),
        };
    });
}

/// Runs a background result against the session once nothing else is
/// using it; a modal dialog can be open while results arrive.
fn apply_when_free(apply: impl FnOnce(&mut Session) + 'static) {
    let Some(session) = ACTIVE.with(|active| active.borrow().upgrade()) else {
        return;
    };
    let Ok(mut borrowed) = session.try_borrow_mut() else {
        slint::Timer::single_shot(Duration::from_millis(20), move || apply_when_free(apply));
        return;
    };
    apply(&mut borrowed);
}

fn sha_error_text(error: Sha256ParseError) -> String {
    let text = error.to_string();
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

fn model<T: Clone + 'static>(items: impl Iterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.collect::<Vec<T>>()))
}

/// Pairs the two columns line by line.
fn details(left: &[Detail], right: &[Detail]) -> ModelRc<DetailRowData> {
    let empty = (String::new(), String::new());
    let lines = left.len().max(right.len());
    model((0..lines).map(|line| {
        let (left_key, left_value) = left.get(line).unwrap_or(&empty);
        let (right_key, right_value) = right.get(line).unwrap_or(&empty);
        DetailRowData {
            left_key: left_key.clone().into(),
            left_value: left_value.clone().into(),
            right_key: right_key.clone().into(),
            right_value: right_value.clone().into(),
        }
    }))
}
