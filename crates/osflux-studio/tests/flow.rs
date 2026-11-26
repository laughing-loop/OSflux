//! Steps 1 to 3 driven through the real window: choosing an image, picking
//! a drive and the safety check, with the stand-in engine's sample images
//! and drives. Each test acts the way a person would, through the window's
//! own callbacks.

mod support;

use osflux_studio::settings::Settings;
use osflux_studio::steps::StepId;
use osflux_studio::write_step::WriteFailure;
use osflux_studio::{HintIcon, MatchState, Screen, Status, StepState};
use slint::Model;
use support::{
    drive, frame_05_drives, studio, studio_with, system_drive, with_unreadable_contents,
    BROKEN_ISO, LINUX_ISO, MISSING_ISO, WINDOWS_ISO, WRITTEN_SHA256,
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

// ---- Windows options and Confirm (frames 09–14, 21) -----------------------

/// Image, drive and a passed safety check, for the Windows ISO on the
/// SanDisk stick (empty, so its code confirms).
fn through_the_safety_check(app: &support::Studio, image: &str, drive: i32) {
    app.open(image);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_select_drive(drive);
    studio.invoke_next();
    assert!(studio.get_can_go_next(), "the safety check passed");
}

#[test]
fn windows_options_are_off_until_ticked_and_shown_everywhere() {
    let app = studio();
    through_the_safety_check(&app, WINDOWS_ISO, 0);
    let studio = app.studio();
    studio.invoke_next();

    assert_eq!(studio.get_screen(), Screen::WindowsOptions);
    assert_eq!(studio.get_step_label(), "Step 4 of 5");
    assert!(!studio.get_option_bypass());
    assert_eq!(
        studio.get_hint(),
        "All options are off. Windows Setup runs as normal."
    );
    assert_eq!(studio.get_summary().windows_options, "None");
    assert_eq!(
        studio.get_steps().row_data(2).unwrap().subtitle,
        "Ready",
        "done steps show the verdict"
    );

    studio.invoke_set_option(0, true);
    studio.invoke_set_option(1, true);
    assert!(!studio.get_can_go_next(), "a local account needs a name");
    studio.invoke_account_name_edited("Administrator".into());
    assert!(!studio.get_can_go_next());
    assert!(!studio.get_account_error().is_empty());
    studio.invoke_account_name_edited("alex".into());
    studio.invoke_set_option(2, true);
    assert!(studio.get_can_go_next());
    assert_eq!(
        studio.get_hint(),
        "3 options are on. They apply to every PC that boots from this stick."
    );
    assert_eq!(
        studio.get_summary().windows_options,
        "Skip hardware checks · Local account (alex) · Skip privacy questions"
    );

    let setup = app.session().borrow().windows_options().setup_options();
    assert!(setup.bypass_requirements && setup.skip_privacy_questions);
    assert_eq!(setup.local_account.as_deref(), Some("alex"));

    studio.invoke_next();
    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert_eq!(
        studio.get_steps().row_data(3).unwrap().subtitle,
        "3 options on"
    );
    assert_eq!(
        studio.get_confirm_options(),
        "Skip hardware checks · Local account (alex) · Skip privacy questions"
    );
}

#[test]
fn an_empty_drive_is_confirmed_with_its_code() {
    let app = studio();
    through_the_safety_check(&app, WINDOWS_ISO, 0);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_next();

    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert_eq!(studio.get_step_label(), "Step 5 of 5");
    assert!(studio.get_confirm_short());
    assert_eq!(studio.get_drive_code(), "7F32");
    assert_eq!(studio.get_confirm_drive().model, "SanDisk Ultra 64GB");
    assert_eq!(studio.get_match_state(), MatchState::Empty);
    assert!(!studio.get_can_write());
    assert_eq!(
        studio.get_steps().row_data(4).unwrap().subtitle,
        "Type the drive code"
    );

    studio.invoke_typed_edited("7f3".into());
    assert_eq!(studio.get_typed(), "7F3", "shown in upper case");
    assert_eq!(studio.get_match_state(), MatchState::Typing);
    studio.invoke_typed_edited("7F3B".into());
    assert_eq!(studio.get_match_state(), MatchState::Mismatch);
    assert_eq!(studio.get_match_message(), "Code does not match this drive");
    assert!(!studio.get_can_write());

    studio.invoke_typed_edited("7f32".into());
    assert_eq!(studio.get_match_state(), MatchState::Match);
    assert_eq!(studio.get_match_message(), "Matches SanDisk Ultra 64GB");
    assert!(studio.get_can_write());
    assert_eq!(
        app.session().borrow().typed(),
        "7F32",
        "kept exactly as shown"
    );

    studio.invoke_write();
    assert_eq!(
        studio.get_screen(),
        Screen::Done,
        "the sample write finishes at once"
    );
    let writes = app.engine.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].typed, "7F32", "the code exactly as typed");
    assert_eq!(writes[0].acknowledgement, None);
    assert!(writes[0].verify);
}

/// An empty stick confirmed with its code, ready to write.
fn confirmed_with_the_code(app: &support::Studio) {
    through_the_safety_check(app, WINDOWS_ISO, 0);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_next();
    studio.invoke_typed_edited("7F32".into());
    assert!(studio.get_can_write());
}

#[test]
fn a_finished_write_is_written_and_verified_as_in_frame_18() {
    let app = studio();
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();

    assert_eq!(studio.get_screen(), Screen::Done);
    assert_eq!(studio.get_step_label(), "Done");
    assert_eq!(studio.get_title(), "Written and verified");
    assert_eq!(studio.get_done_status(), Status::Ready);
    assert!(!studio.get_writing());
    let rows = studio.get_done_rows();
    assert_eq!(
        rows.row_data(0).unwrap().value,
        WRITTEN_SHA256.to_ascii_uppercase()
    );
    assert_eq!(rows.row_data(1).unwrap().value, "Not entered");
    assert_eq!(rows.row_data(2).unwrap().status, Status::Ready);
    assert_eq!(rows.row_data(3).unwrap().value, "15 min 02 s");
    assert!(studio.get_done_next().contains("UEFI: entry"));
    assert_eq!(studio.get_done_primary_label(), "Write another stick");
    let confirm = studio.get_steps().row_data(4).unwrap();
    assert_eq!(confirm.state, StepState::Done);
    assert_eq!(confirm.subtitle, "Written and verified");
}

#[test]
fn the_expected_hash_is_compared_after_writing() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_expected_sha_edited(WRITTEN_SHA256.into());
    studio.invoke_next();
    studio.invoke_select_drive(0);
    studio.invoke_next();
    studio.invoke_next();
    studio.invoke_next();
    studio.invoke_typed_edited("7F32".into());
    studio.invoke_write();

    assert_eq!(studio.get_title(), "Written and verified");
    let expected = studio.get_done_rows().row_data(1).unwrap();
    assert_eq!(expected.status, Status::Ready);
    assert_eq!(expected.status_text, "Matches");
}

#[test]
fn a_different_expected_hash_is_a_mismatch_as_in_frame_20() {
    let app = studio();
    app.open(WINDOWS_ISO);
    let studio = app.studio();
    studio.invoke_expected_sha_edited(
        "A1ACC315D73B2A9CDF67FECB16403D751C99B6E50983382663FE0745CDFCDB13".into(),
    );
    studio.invoke_next();
    studio.invoke_select_drive(0);
    studio.invoke_next();
    studio.invoke_next();
    studio.invoke_next();
    studio.invoke_typed_edited("7F32".into());
    studio.invoke_write();

    assert_eq!(studio.get_title(), "Hash mismatch");
    assert_eq!(studio.get_done_status(), Status::Blocked);
    assert!(studio.get_done_band_red());
    assert_eq!(
        studio.get_done_band_title(),
        "Download the image again before you use this stick."
    );
    assert!(studio.get_done_rows().row_data(1).unwrap().alert);
    let confirm = studio.get_steps().row_data(4).unwrap();
    assert_eq!(confirm.state, StepState::Blocked);
    assert_eq!(confirm.subtitle, "Hash mismatch");
}

#[test]
fn with_verification_off_the_stick_is_written_not_verified_as_in_frame_19() {
    let app = studio();
    app.studio().invoke_set_verify_after_writing(false);
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();

    assert!(!app.engine.writes.lock().unwrap()[0].verify);
    assert_eq!(studio.get_title(), "Written, not verified");
    assert_eq!(studio.get_done_status(), Status::NeedsInspection);
}

#[test]
fn a_drive_with_files_is_written_with_the_acknowledgement() {
    let app = studio();
    through_the_safety_check(&app, LINUX_ISO, 2);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_typed_edited("7F34".into());
    studio.set_acknowledged(true);
    studio.invoke_acknowledged_changed(true);
    studio.invoke_write();

    let writes = app.engine.writes.lock().unwrap();
    assert_eq!(writes[0].typed, "7F34");
    assert_eq!(
        writes[0].acknowledgement.as_deref(),
        Some("Erase Generic SD/MMC USB Device (14.8 GB) and the 1 partition with files on it")
    );
    assert!(writes[0].setup.is_empty(), "no Windows options for Linux");
    assert!(studio.get_done_next().contains("choose the USB stick"));
}

#[test]
fn windows_options_go_with_the_write() {
    let app = studio();
    through_the_safety_check(&app, WINDOWS_ISO, 0);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_set_option(0, true);
    studio.invoke_next();
    studio.invoke_typed_edited("7F32".into());
    studio.invoke_write();

    let writes = app.engine.writes.lock().unwrap();
    assert!(writes[0].setup.bypass_requirements);
    assert!(!writes[0].copy_region);
}

#[test]
fn a_cancelled_write_offers_to_try_again() {
    let app = studio();
    *app.engine.write_failure.lock().unwrap() = Some(WriteFailure::Cancelled { written: true });
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();

    assert_eq!(studio.get_screen(), Screen::Done);
    assert_eq!(studio.get_title(), "Writing cancelled");
    assert_eq!(
        studio.get_done_band_title(),
        "The stick needs writing again before you can use it."
    );
    assert!(!studio.get_done_band_red(), "amber, not red");
    assert_eq!(studio.get_done_primary_label(), "Try again");

    studio.invoke_done_primary();
    assert_eq!(
        studio.get_screen(),
        Screen::Confirm,
        "confirm the drive again"
    );
    assert!(!studio.get_can_write());
}

#[test]
fn an_expired_confirmation_at_write_time_shows_frame_21() {
    let app = studio();
    *app.engine.write_failure.lock().unwrap() = Some(WriteFailure::Expired);
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();

    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert!(studio.get_confirm_expired());
}

#[test]
fn a_drive_that_changed_at_write_time_shows_frame_22() {
    let app = studio();
    *app.engine.write_failure.lock().unwrap() =
        Some(WriteFailure::DriveChanged("the drive changed".into()));
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();

    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert!(studio.get_confirm_error().contains("The drive changed"));
}

#[test]
fn a_failed_write_says_why() {
    let app = studio();
    *app.engine.write_failure.lock().unwrap() =
        Some(WriteFailure::InUse("a volume is in use".into()));
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();

    assert_eq!(studio.get_title(), "Not written");
    assert_eq!(
        studio.get_done_band_title(),
        "Another program is using the drive."
    );
    assert!(studio.get_done_band_red());
}

#[test]
fn write_another_stick_goes_back_to_the_drives() {
    let app = studio();
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.invoke_write();
    studio.invoke_done_primary();

    assert_eq!(studio.get_screen(), Screen::Drive);
    assert_eq!(
        studio.get_steps().row_data(1).unwrap().subtitle,
        "Choose one drive"
    );
    assert!(!studio.get_can_go_next(), "no drive chosen yet");
}

#[test]
fn without_administrator_rights_writing_is_locked_as_in_frame_23() {
    let app = studio();
    confirmed_with_the_code(&app);
    let studio = app.studio();
    studio.set_administrator(false);
    studio.invoke_typed_edited("7F32".into());

    assert!(!studio.get_can_write());
    assert_eq!(
        studio.get_hint(),
        "Write to drive needs Administrator rights."
    );
    studio.invoke_write();
    assert_eq!(studio.get_screen(), Screen::Confirm, "nothing starts");
    assert!(app.engine.writes.lock().unwrap().is_empty());
}

#[test]
fn a_drive_with_files_is_confirmed_with_its_code_and_the_box() {
    let app = studio();
    through_the_safety_check(&app, LINUX_ISO, 2);
    let studio = app.studio();
    studio.invoke_next();

    assert_eq!(
        studio.get_screen(),
        Screen::Confirm,
        "no Windows options for Linux"
    );
    assert!(!studio.get_confirm_short());
    assert!(studio.get_confirm_acknowledge());
    assert_eq!(studio.get_drive_code(), "7F34");
    assert_eq!(
        studio.get_acknowledgement(),
        "Erase Generic SD/MMC USB Device (14.8 GB) and the 1 partition with files on it"
    );
    assert_eq!(
        studio.get_contents_note(),
        "This drive has files on it. Type its code and tick the box to confirm."
    );
    assert!(studio
        .get_erase_title()
        .contains("Generic SD/MMC USB Device"));
    assert!(!studio.get_acknowledged());

    studio.invoke_typed_edited("7f34".into());
    assert_eq!(studio.get_typed(), "7F34", "kept in the code boxes");
    assert_eq!(studio.get_match_state(), MatchState::Typing);
    assert_eq!(
        studio.get_match_message(),
        "Code matches. Tick the box to unlock Write to drive."
    );
    assert!(!studio.get_can_write(), "the code alone is not enough");

    studio.set_acknowledged(true);
    studio.invoke_acknowledged_changed(true);
    assert_eq!(studio.get_match_state(), MatchState::Match);
    assert_eq!(
        studio.get_match_message(),
        "Matches Generic SD/MMC USB Device"
    );
    assert!(studio.get_can_write());

    studio.invoke_typed_edited("7F35".into());
    assert_eq!(studio.get_match_state(), MatchState::Mismatch);
    assert!(
        !studio.get_can_write(),
        "the box does not make a wrong code right"
    );

    studio.invoke_typed_edited("7F34".into());
    studio.set_acknowledged(false);
    studio.invoke_acknowledged_changed(false);
    assert!(!studio.get_can_write(), "clearing the box locks it again");
}

#[test]
fn a_drive_whose_contents_cannot_be_read_needs_the_full_phrase() {
    let mut drives = frame_05_drives();
    let last = drives.len() - 1;
    drives[last] = with_unreadable_contents(drives[last].clone());
    let app = studio_with(Settings::default(), drives);
    through_the_safety_check(&app, LINUX_ISO, 2);
    let studio = app.studio();
    studio.invoke_next();

    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert!(!studio.get_confirm_short());
    assert!(!studio.get_confirm_acknowledge());
    assert_eq!(studio.get_phrase_head(), "OSFLUX ERASE TARGET 4 ");
    assert_eq!(studio.get_phrase_code(), "7F34");
    assert_eq!(
        studio.get_contents_note(),
        "OSflux couldn't check what's on this drive, so it asks for the full phrase."
    );
    assert!(studio
        .get_erase_title()
        .contains("Generic SD/MMC USB Device"));

    studio.invoke_typed_edited("7F34".into());
    assert_eq!(
        studio.get_match_state(),
        MatchState::Mismatch,
        "the code alone is not enough"
    );
    studio.invoke_typed_edited("osflux erase target 4".into());
    assert_eq!(studio.get_match_state(), MatchState::Typing);
    assert_eq!(studio.get_match_message(), "Keep typing…");
    studio.invoke_typed_edited("OSFLUX ERASE TARGET 4 7F34".into());
    assert_eq!(studio.get_match_state(), MatchState::Match);
    assert!(studio.get_can_write());
}

#[test]
fn an_expired_confirmation_locks_writing_until_confirmed_again() {
    let app = studio();
    through_the_safety_check(&app, WINDOWS_ISO, 0);
    let studio = app.studio();
    studio.invoke_next();
    studio.invoke_next();
    studio.invoke_typed_edited("7F32".into());
    assert!(studio.get_can_write());

    // Ten minutes pass.
    *app.engine.now.lock().unwrap() = support::START_TIME + 600;
    app.session()
        .borrow_mut()
        .check_expiry(support::START_TIME + 600);
    assert!(studio.get_confirm_expired());
    assert!(
        !studio.get_can_write(),
        "a matching code no longer unlocks it"
    );
    assert_eq!(
        studio.get_hint(),
        "Write to drive is locked until you confirm again."
    );
    assert_eq!(
        studio.get_steps().row_data(4).unwrap().subtitle,
        "Confirmation expired"
    );

    studio.invoke_confirm_again();
    assert!(!studio.get_confirm_expired());
    assert_eq!(studio.get_typed(), "", "the code is typed again");
    assert!(!studio.get_can_write());
}

#[test]
fn a_drive_swapped_before_confirming_is_refused() {
    let app = studio();
    through_the_safety_check(&app, LINUX_ISO, 0);
    let mut other = drive(
        2,
        "Another stick",
        64_023_511_040,
        osflux_engine::TargetAssessmentStatus::Eligible,
        "target is empty",
    );
    other.identity_code = Some("0BAD".to_string());
    *app.engine.drives.lock().unwrap() = vec![other];
    let studio = app.studio();
    studio.invoke_next();

    assert_eq!(studio.get_screen(), Screen::Confirm);
    assert!(studio
        .get_confirm_error()
        .starts_with("The drive changed after the safety check"));
    assert!(!studio.get_can_write());
}
