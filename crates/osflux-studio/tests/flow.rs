//! Steps 1 to 3 driven through the real window: choosing an image, picking
//! a drive and the safety check, with the stand-in engine's sample images
//! and drives. Each test acts the way a person would, through the window's
//! own callbacks.

mod support;

use osflux_studio::settings::Settings;
use osflux_studio::steps::StepId;
use osflux_studio::{HintIcon, Screen, Status, StepState};
use slint::Model;
use support::{
    drive, frame_05_drives, studio, studio_with, system_drive, BROKEN_ISO, LINUX_ISO, MISSING_ISO,
    WINDOWS_ISO,
};

fn rail_titles(studio: &osflux_studio::Studio<'_>) -> Vec<String> {
    studio
        .get_steps()
        .iter()
        .map(|step| step.title.to_string())
        .collect()
}

#[test]
fn a_windows_image_shows_its_plan_and_five_steps() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();

    assert_eq!(studio.get_screen(), Screen::Image);
    assert!(studio.get_image_chosen());
    assert_eq!(studio.get_title(), "Image checked");
    assert_eq!(studio.get_step_label(), "Step 1 of 5");
    assert_eq!(studio.get_image_size(), "8.41 GB");
    assert_eq!(studio.get_plan_status(), Status::Ready);
    assert_eq!(studio.get_plan_rows().row_count(), 5);
    assert_eq!(
        studio.get_plan_rows().row_data(3).unwrap().value,
        "Windows installer: files copied to a FAT32 stick (UEFI)"
    );
    assert!(studio.get_can_go_next());
    assert!(!studio.get_offer_another_image());
    assert_eq!(
        rail_titles(&studio),
        [
            "Image",
            "Drive",
            "Safety check",
            "Windows options",
            "Confirm"
        ]
    );
    let summary = studio.get_summary();
    assert_eq!(summary.image, "Windows11_Client_…_9457.iso");
    assert_eq!(
        summary.image_full,
        "Windows11_Client_x64_en-gb_26300_9457.iso"
    );
    assert!(summary.show_windows_options);
    assert_eq!(summary.windows_options, "None");
}

#[test]
fn a_linux_image_has_four_steps() {
    let app = studio();
    app.open(LINUX_ISO);
    let studio = app.studio();
    assert_eq!(studio.get_step_label(), "Step 1 of 4");
    assert_eq!(
        rail_titles(&studio),
        ["Image", "Drive", "Safety check", "Confirm"]
    );
    assert!(!studio.get_summary().show_windows_options);
}

#[test]
fn an_image_needing_inspection_offers_another_and_locks_next() {
    let app = studio();
    app.open(BROKEN_ISO);
    let studio = app.studio();
    assert_eq!(studio.get_title(), "Image needs a closer look");
    assert_eq!(studio.get_plan_status(), Status::NeedsInspection);
    assert_eq!(studio.get_hint(), "This image can't be written yet.");
    assert_eq!(studio.get_hint_icon(), HintIcon::Info);
    assert!(!studio.get_can_go_next());
    assert!(studio.get_offer_another_image());

    studio.invoke_next();
    assert_eq!(studio.get_screen(), Screen::Image, "Next stays locked");
}

#[test]
fn an_unreadable_image_says_why_and_stays_on_step_1() {
    let app = studio();
    app.open(MISSING_ISO);
    let studio = app.studio();
    assert!(!studio.get_image_chosen());
    assert!(studio
        .get_image_error()
        .starts_with("OSflux couldn't read missing.iso: image path does not exist"));
    assert!(!studio.get_can_go_next());
}

#[test]
fn the_drive_step_lists_allowed_and_blocked_drives() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();

    assert_eq!(studio.get_screen(), Screen::Drive);
    assert_eq!(studio.get_title(), "Choose a drive");
    assert_eq!(studio.get_step_label(), "Step 2 of 5");
    assert_eq!(studio.get_image_needs(), "8.41 GB");
    let drives = studio.get_drives();
    assert_eq!(drives.row_count(), 3);
    assert_eq!(drives.row_data(0).unwrap().model, "SanDisk Ultra 64GB");
    assert_eq!(drives.row_data(0).unwrap().size, "59.6 GB");
    assert_eq!(drives.row_data(2).unwrap().status_detail, "Has files on it");
    assert_eq!(studio.get_blocked_drives().row_count(), 1);
    assert_eq!(
        studio.get_blocked_drives().row_data(0).unwrap().reason,
        if cfg!(windows) {
            "System drive — Windows is installed here"
        } else {
            "System drive — this computer runs from it"
        }
    );
    assert!(studio.get_blocked_open(), "frame 05 shows Not allowed open");
    assert!(!studio.get_can_go_next(), "nothing picked yet");
    assert_eq!(
        studio.get_steps().row_data(1).unwrap().subtitle,
        "Choose one drive"
    );

    studio.invoke_select_drive(0);
    assert!(studio.get_drives().row_data(0).unwrap().selected);
    assert_eq!(studio.get_hint(), "Selected: SanDisk Ultra 64GB");
    assert_eq!(studio.get_summary().drive, "SanDisk Ultra 64GB");
    assert!(studio.get_can_go_next());
}

#[test]
fn with_no_allowed_drives_the_empty_state_shows() {
    let app = studio_with(Settings::default(), vec![system_drive()]);
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();

    assert_eq!(studio.get_drives().row_count(), 0);
    assert!(studio.get_drives_listed());
    assert!(
        !studio.get_blocked_open(),
        "frame 06 keeps Not allowed closed"
    );
    assert_eq!(studio.get_hint(), "Waiting for a USB stick or SD card.");
    assert_eq!(
        studio.get_steps().row_data(1).unwrap().subtitle,
        "No drive yet"
    );
    assert!(!studio.get_can_go_next());
}

#[test]
fn a_drive_that_fits_passes_the_safety_check() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(0);
    studio.invoke_next();

    assert_eq!(studio.get_screen(), Screen::Safety);
    assert_eq!(
        studio.get_step_label(),
        "Step 3 of 5 · Safety check verdict"
    );
    assert_eq!(studio.get_title(), "Ready");
    assert_eq!(studio.get_verdict(), Status::Ready);
    assert_eq!(studio.get_checks().row_count(), 5);
    assert_eq!(studio.get_checks_passed(), "5 of 5 checks passed");
    assert_eq!(studio.get_fix_label(), "");
    assert!(studio.get_details_open());
    assert_eq!(studio.get_hint(), "All checks passed.");
    let rail = studio.get_steps();
    assert_eq!(rail.row_data(1).unwrap().subtitle, "SanDisk Ultra 64GB");
    assert_eq!(rail.row_data(2).unwrap().subtitle, "Verdict: Ready");
    assert!(studio.get_can_go_next());

    // Windows options and Confirm come in the next pull request.
    studio.invoke_next();
    assert_eq!(studio.get_screen(), Screen::WindowsOptions);
    assert_eq!(studio.get_title(), "Windows options");
    studio.invoke_next();
    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert!(!studio.get_can_go_next());
    studio.invoke_back();
    assert_eq!(studio.get_screen(), Screen::WindowsOptions);
}

#[test]
fn a_drive_that_is_too_small_is_blocked_with_a_way_back() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(1);
    studio.invoke_next();

    assert_eq!(studio.get_title(), "Blocked");
    assert_eq!(studio.get_verdict(), Status::Blocked);
    assert!(studio
        .get_description()
        .contains("It needs 8.41 GB, but this drive holds 7.45 GB."));
    assert_eq!(studio.get_fix_label(), "Choose another drive");
    assert_eq!(studio.get_checks_passed(), "");
    assert!(!studio.get_details_open(), "frame 08 folds the details");
    assert!(!studio.get_can_go_next());
    let safety_step = studio.get_steps().row_data(2).unwrap();
    assert_eq!(safety_step.state, StepState::Blocked);
    assert_eq!(safety_step.subtitle, "Blocked: image too large");

    studio.invoke_fix();
    assert_eq!(studio.get_screen(), Screen::Drive);
}

#[test]
fn a_drive_with_files_needs_confirmation() {
    let app = studio();
    app.open(LINUX_ISO);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(2);
    studio.invoke_next();
    assert_eq!(studio.get_verdict(), Status::NeedsConfirmation);
    assert_eq!(studio.get_title(), "Needs confirmation");
    assert!(studio.get_can_go_next());
    studio.invoke_next();
    assert_eq!(
        studio.get_screen(),
        Screen::Confirm,
        "Linux images skip Windows options"
    );
}

#[test]
fn turning_verification_off_shows_on_the_safety_check() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(0);
    studio.invoke_next();

    studio.invoke_set_verify_after_writing(false);
    let verification = studio.get_checks().row_data(4).unwrap();
    assert_eq!(verification.status, Status::NeedsInspection);
    assert_eq!(verification.value, "off");
    assert_eq!(studio.get_checks_passed(), "4 of 5 checks passed");
    assert!(!studio.get_summary().image.is_empty());
}

#[test]
fn a_drive_pulled_out_before_the_check_is_reported() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(0);

    // Unplugged while still on the Drive step: the choice is dropped.
    *app.engine.drives.lock().unwrap() = vec![system_drive()];
    studio.invoke_refresh();
    assert!(app.session().borrow().chosen_drive().is_none());
    assert!(!studio.get_can_go_next());

    // Plugged back in and chosen, then swapped for another stick in the
    // same port before the check: the engine sees a different drive.
    *app.engine.drives.lock().unwrap() = frame_05_drives();
    studio.invoke_refresh();
    studio.invoke_select_drive(0);
    let mut other = drive(
        2,
        "Another stick",
        64_023_511_040,
        osflux_engine::TargetAssessmentStatus::Eligible,
        "target is empty",
    );
    other.identity_code = Some("0BAD".to_string());
    *app.engine.drives.lock().unwrap() = vec![other];
    studio.invoke_next();
    assert_eq!(studio.get_verdict(), Status::Blocked);
    assert_eq!(
        studio.get_steps().row_data(2).unwrap().subtitle,
        "Blocked: drive changed"
    );
    assert_eq!(studio.get_fix_label(), "Choose another drive");
}

#[test]
fn the_expected_sha256_is_checked_as_it_is_typed() {
    let app = studio();
    app.open(LINUX_ISO);
    let studio = app.studio();

    studio.invoke_expected_sha_edited("12ab".into());
    assert_eq!(studio.get_expected_sha(), "12AB", "shown in upper case");
    assert_eq!(
        studio.get_sha_error(),
        "A SHA-256 has 64 characters; this has 4."
    );
    assert!(app.session().borrow().expected_sha().is_none());

    studio.invoke_expected_sha_edited("zz".into());
    assert_eq!(
        studio.get_sha_error(),
        "A SHA-256 uses only the characters 0–9 and A–F."
    );

    let hash = "2a1ab0d46d7285f738d082858925d3303f3dc430f0a4eab5aa238325a4274906";
    studio.invoke_expected_sha_edited(hash.into());
    assert_eq!(studio.get_sha_error(), "");
    assert_eq!(
        app.session()
            .borrow()
            .expected_sha()
            .map(|hash| hash.as_str().to_string()),
        Some(hash.to_string())
    );

    studio.invoke_expected_sha_edited("".into());
    assert!(app.session().borrow().expected_sha().is_none());

    // A new image starts with an empty field.
    studio.invoke_expected_sha_edited(hash.into());
    app.open(WINDOWS_ISO);
    assert_eq!(studio.get_expected_sha(), "");
    assert!(app.session().borrow().expected_sha().is_none());
}

#[test]
fn done_steps_in_the_rail_go_back_to_them() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(0);
    studio.invoke_next();
    assert!(studio.get_steps().row_data(0).unwrap().clickable);

    // Later steps can't be jumped to.
    studio.invoke_go_to_step(4);
    assert_eq!(studio.get_screen(), Screen::Safety);

    studio.invoke_go_to_step(1);
    assert_eq!(studio.get_screen(), Screen::Drive);
    assert!(
        studio.get_drives().row_data(0).unwrap().selected,
        "the choice is kept"
    );
    studio.invoke_go_to_step(0);
    assert_eq!(studio.get_screen(), Screen::Image);
    assert_eq!(app.session().borrow().step(), StepId::Image);
}
