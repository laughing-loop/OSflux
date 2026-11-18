use std::io::{IsTerminal, Write};

use osflux_engine::{
    assess_target, build_image_plan_report, create_write_intent, detect_image_family,
    format_unix_utc, list_devices, now_unix_seconds, plan_image, plan_iso_filesystem,
    plan_write_chunks, prepare_raw_write, prepare_write_command, prepare_write_session,
    preview_write_progress, record_write_authorization, run_write_preflight, select_write_mode,
    write_image_to_target, AuditEvent, AuditEventKind, AuditLog, AuthorizationStore, ChunkPlan,
    ChunkPlanStatus, Engine, EngineConfig, ImageFamilyReport, ImageFamilyStatus, ImagePlanStatus,
    IsoFilesystemPlan, IsoFilesystemPlanStatus, PlanReport, ProgressPreview, ProgressPreviewStatus,
    RawWriteError, RawWritePhase, RawWritePlan, RawWriteProgress, RawWriteReport, TargetAssessment,
    TargetAssessmentStatus, WriteCommandPlan, WriteCommandStatus, WriteIntent, WriteIntentStatus,
    WriteModeSelection, WriteModeSelectionStatus, WritePreflight, WritePreflightStatus,
    WriteSession, WriteSessionStatus, DEFAULT_CHUNK_SIZE_BYTES,
};

/// Exit code for a command that ran but whose result is blocked or needs inspection.
const EXIT_NOT_READY: i32 = 3;

fn main() {
    match parse_command(std::env::args().skip(1)) {
        Command::Run(config) => {
            println!("OSflux CLI");
            let engine = Engine::new(config);
            engine.start();
        }
        Command::ListDevices => print_devices(),
        Command::PlanImage { path, output } => print_image_plan(&path, output),
        Command::DetectFamily { path, output } => print_image_family(&path, output),
        Command::FilesystemPlan { path, output } => print_iso_filesystem_plan(&path, output),
        Command::AssessTarget { index, output } => print_target_assessment(index, output),
        Command::Preflight {
            image,
            target,
            output,
        } => print_preflight(&image, target, output),
        Command::Intent {
            image,
            target,
            output,
        } => print_write_intent(&image, target, output),
        Command::Write {
            image,
            target,
            confirm,
            chunk_size,
            execute,
            assume_yes,
            output,
        } => {
            if execute {
                run_raw_write(
                    &image,
                    target,
                    confirm.as_deref(),
                    chunk_size,
                    assume_yes,
                    output,
                );
            } else {
                print_write_command(&image, target, confirm.as_deref(), output);
            }
        }
        Command::Session {
            image,
            target,
            confirm,
            output,
        } => print_write_session(&image, target, confirm.as_deref(), output),
        Command::ChunkPlan {
            image,
            target,
            confirm,
            chunk_size,
            output,
        } => print_chunk_plan(&image, target, confirm.as_deref(), chunk_size, output),
        Command::ProgressPreview {
            image,
            target,
            confirm,
            chunk_size,
            output,
        } => print_progress_preview(&image, target, confirm.as_deref(), chunk_size, output),
        Command::ModeSelection {
            image,
            target,
            confirm,
            chunk_size,
            output,
        } => print_mode_selection(&image, target, confirm.as_deref(), chunk_size, output),
    }
}

enum Command {
    Run(EngineConfig),
    ListDevices,
    PlanImage {
        path: String,
        output: OutputFormat,
    },
    DetectFamily {
        path: String,
        output: OutputFormat,
    },
    FilesystemPlan {
        path: String,
        output: OutputFormat,
    },
    AssessTarget {
        index: u32,
        output: OutputFormat,
    },
    Preflight {
        image: String,
        target: u32,
        output: OutputFormat,
    },
    Intent {
        image: String,
        target: u32,
        output: OutputFormat,
    },
    Write {
        image: String,
        target: u32,
        confirm: Option<String>,
        chunk_size: u64,
        execute: bool,
        assume_yes: bool,
        output: OutputFormat,
    },
    Session {
        image: String,
        target: u32,
        confirm: Option<String>,
        output: OutputFormat,
    },
    ChunkPlan {
        image: String,
        target: u32,
        confirm: Option<String>,
        chunk_size: u64,
        output: OutputFormat,
    },
    ProgressPreview {
        image: String,
        target: u32,
        confirm: Option<String>,
        chunk_size: u64,
        output: OutputFormat,
    },
    ModeSelection {
        image: String,
        target: u32,
        confirm: Option<String>,
        chunk_size: u64,
        output: OutputFormat,
    },
}

#[derive(Clone, Copy)]
enum OutputFormat {
    Text,
    Json,
}

fn parse_command<I>(args: I) -> Command
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut config = EngineConfig::default();
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    let mut index = 0usize;
    let mut output = OutputFormat::Text;
    let mut plan_image_path = None;
    let mut target_index = None;
    let mut preflight_requested = false;
    let mut intent_requested = false;
    let mut write_requested = false;
    let mut execute_requested = false;
    let mut assume_yes = false;
    let mut session_requested = false;
    let mut chunk_plan_requested = false;
    let mut progress_preview_requested = false;
    let mut mode_selection_requested = false;
    let mut family_detection_requested = false;
    let mut filesystem_plan_requested = false;
    let mut preflight_image = None;
    let mut preflight_target = None;
    let mut confirmation_phrase = None;
    let mut chunk_size_bytes = DEFAULT_CHUNK_SIZE_BYTES;

    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--help" || arg == "-h" {
            print_help();
            std::process::exit(0);
        } else if arg == "--json" {
            output = OutputFormat::Json;
        } else if arg == "--preflight" {
            preflight_requested = true;
        } else if arg == "--intent" {
            intent_requested = true;
        } else if arg == "--write" {
            write_requested = true;
        } else if arg == "--execute" {
            execute_requested = true;
        } else if arg == "--yes" {
            assume_yes = true;
        } else if arg == "--session" {
            session_requested = true;
        } else if arg == "--chunk-plan" {
            chunk_plan_requested = true;
        } else if arg == "--progress-preview" {
            progress_preview_requested = true;
        } else if arg == "--select-mode" {
            mode_selection_requested = true;
        } else if let Some(path) = arg.strip_prefix("--detect-family=") {
            family_detection_requested = true;
            preflight_image = Some(path.to_string());
        } else if arg == "--detect-family" {
            family_detection_requested = true;
        } else if let Some(path) = arg.strip_prefix("--filesystem-plan=") {
            filesystem_plan_requested = true;
            preflight_image = Some(path.to_string());
        } else if arg == "--filesystem-plan" {
            filesystem_plan_requested = true;
        } else if let Some(path) = arg.strip_prefix("--image=") {
            preflight_image = Some(path.to_string());
        } else if arg == "--image" {
            let Some(path) = args.get(index + 1) else {
                eprintln!("missing path after --image");
                std::process::exit(2);
            };

            preflight_image = Some(path.to_string());
            index += 1;
        } else if let Some(value) = arg.strip_prefix("--target=") {
            preflight_target = Some(parse_target_index(value));
        } else if arg == "--target" {
            let Some(value) = args.get(index + 1) else {
                eprintln!("missing index after --target");
                std::process::exit(2);
            };

            preflight_target = Some(parse_target_index(value));
            index += 1;
        } else if let Some(value) = arg.strip_prefix("--confirm=") {
            confirmation_phrase = Some(value.to_string());
        } else if arg == "--confirm" {
            let Some(value) = args.get(index + 1) else {
                eprintln!("missing phrase after --confirm");
                std::process::exit(2);
            };

            confirmation_phrase = Some(value.to_string());
            index += 1;
        } else if let Some(value) = arg.strip_prefix("--chunk-size=") {
            chunk_size_bytes = parse_u64_option(value, "chunk size");
        } else if arg == "--chunk-size" {
            let Some(value) = args.get(index + 1) else {
                eprintln!("missing byte count after --chunk-size");
                std::process::exit(2);
            };

            chunk_size_bytes = parse_u64_option(value, "chunk size");
            index += 1;
        } else if arg == "--list-devices" {
            return Command::ListDevices;
        } else if let Some(value) = arg.strip_prefix("--assess-target=") {
            target_index = Some(parse_target_index(value));
        } else if arg == "--assess-target" {
            let Some(value) = args.get(index + 1) else {
                eprintln!("missing index after --assess-target");
                std::process::exit(2);
            };

            target_index = Some(parse_target_index(value));
            index += 1;
        } else if let Some(path) = arg.strip_prefix("--plan-image=") {
            plan_image_path = Some(path.to_string());
        } else if arg == "--plan-image" {
            let Some(path) = args.get(index + 1) else {
                eprintln!("missing path after --plan-image");
                std::process::exit(2);
            };

            plan_image_path = Some(path.to_string());
            index += 1;
        } else if let Some(value) = arg.strip_prefix("--blocks=") {
            config.block_count = parse_usize_option(value, "block count");
        } else if let Some(value) = arg.strip_prefix("--block-size=") {
            config.block_size = parse_usize_option(value, "block size");
        } else {
            eprintln!("unknown argument: {arg}");
            eprintln!("run with --help to see the supported options");
            std::process::exit(2);
        }

        index += 1;
    }

    let other_mode_requested = mode_selection_requested
        || progress_preview_requested
        || chunk_plan_requested
        || session_requested;
    if execute_requested && (!write_requested || other_mode_requested) {
        eprintln!("--execute can only be used with --write");
        std::process::exit(2);
    }
    if assume_yes && !execute_requested {
        eprintln!("--yes can only be used with --write --execute");
        std::process::exit(2);
    }

    if mode_selection_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --select-mode");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --select-mode");
            std::process::exit(2);
        };

        return Command::ModeSelection {
            image,
            target,
            confirm: confirmation_phrase,
            chunk_size: chunk_size_bytes,
            output,
        };
    }

    if progress_preview_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --progress-preview");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --progress-preview");
            std::process::exit(2);
        };

        return Command::ProgressPreview {
            image,
            target,
            confirm: confirmation_phrase,
            chunk_size: chunk_size_bytes,
            output,
        };
    }

    if chunk_plan_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --chunk-plan");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --chunk-plan");
            std::process::exit(2);
        };

        return Command::ChunkPlan {
            image,
            target,
            confirm: confirmation_phrase,
            chunk_size: chunk_size_bytes,
            output,
        };
    }

    if session_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --session");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --session");
            std::process::exit(2);
        };

        return Command::Session {
            image,
            target,
            confirm: confirmation_phrase,
            output,
        };
    }

    if write_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --write");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --write");
            std::process::exit(2);
        };

        return Command::Write {
            image,
            target,
            confirm: confirmation_phrase,
            chunk_size: chunk_size_bytes,
            execute: execute_requested,
            assume_yes,
            output,
        };
    }

    if intent_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --intent");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --intent");
            std::process::exit(2);
        };

        return Command::Intent {
            image,
            target,
            output,
        };
    }

    if preflight_requested {
        let Some(image) = preflight_image else {
            eprintln!("missing --image <PATH> for --preflight");
            std::process::exit(2);
        };
        let Some(target) = preflight_target else {
            eprintln!("missing --target <INDEX> for --preflight");
            std::process::exit(2);
        };

        return Command::Preflight {
            image,
            target,
            output,
        };
    }

    if family_detection_requested {
        let Some(path) = preflight_image else {
            eprintln!("missing --image <PATH> for --detect-family");
            std::process::exit(2);
        };

        return Command::DetectFamily { path, output };
    }

    if filesystem_plan_requested {
        let Some(path) = preflight_image else {
            eprintln!("missing --image <PATH> for --filesystem-plan");
            std::process::exit(2);
        };

        return Command::FilesystemPlan { path, output };
    }

    if let Some(path) = plan_image_path {
        return Command::PlanImage { path, output };
    }

    if let Some(index) = target_index {
        return Command::AssessTarget { index, output };
    }

    Command::Run(config)
}

fn print_image_family(path: &str, output: OutputFormat) {
    match detect_image_family(path) {
        Ok(report) => {
            let ready = report.status == ImageFamilyStatus::Detected;
            match output {
                OutputFormat::Text => print_image_family_report(report),
                OutputFormat::Json => print!("{}", report.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not detect this image family.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_image_family_report(report: ImageFamilyReport) {
    println!("OSflux image family");
    println!("status: {}", report.status.as_str());
    println!("family: {}", report.family.as_str());
    println!("reason: {}", report.reason);
    println!("image: {}", report.image_source);
    println!("image kind: {}", report.image_kind.as_str());
    println!("image status: {}", report.image_status.as_str());
    println!("detection method: {}", report.detection_method.as_str());
    println!(
        "scanned bytes: {} ({})",
        report.scanned_bytes,
        format_size(report.scanned_bytes)
    );
    println!(
        "scan limit: {} ({})",
        report.scan_limit_bytes,
        format_size(report.scan_limit_bytes)
    );

    if !report.evidence.is_empty() {
        println!("evidence:");
        for item in &report.evidence {
            println!(
                "  - {} | {} | {}",
                item.family.as_str(),
                item.marker,
                item.description
            );
        }
    }

    if !report.notes.is_empty() {
        println!("notes:");
        for note in &report.notes {
            println!("  - {note}");
        }
    }

    if !report.warnings.is_empty() {
        println!("warnings:");
        for warning in &report.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_iso_filesystem_plan(path: &str, output: OutputFormat) {
    match plan_iso_filesystem(path) {
        Ok(plan) => {
            let ready = plan.status == IsoFilesystemPlanStatus::Planned;
            match output {
                OutputFormat::Text => print_iso_filesystem_plan_report(plan),
                OutputFormat::Json => print!("{}", plan.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not build an ISO filesystem plan.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_iso_filesystem_plan_report(plan: IsoFilesystemPlan) {
    println!("OSflux ISO filesystem plan");
    println!("status: {}", plan.status.as_str());
    println!("strategy: {}", plan.strategy.as_str());
    println!("reason: {}", plan.reason);
    println!("image: {}", plan.image_source);
    println!("image status: {}", plan.image_status.as_str());
    println!("detected family: {}", plan.detected_family.as_str());
    println!("planned family: {}", plan.planned_family.as_str());
    println!("boot style: {}", plan.boot_style.as_str());
    println!(
        "valid boot catalog: {}",
        if plan.capabilities.has_valid_boot_catalog() {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "directory scanned: {}",
        if plan.capabilities.directory_scanned {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "directory truncated: {}",
        if plan.capabilities.directory_truncated {
            "yes"
        } else {
            "no"
        }
    );
    println!("boot assets: {}", plan.capabilities.boot_asset_count);
    println!("windows assets: {}", plan.capabilities.windows_asset_count);
    println!("linux assets: {}", plan.capabilities.linux_asset_count);
    println!(
        "has installer image: {}",
        if plan.capabilities.has_installer_image {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "has linux kernel: {}",
        if plan.capabilities.has_linux_kernel {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "has initrd: {}",
        if plan.capabilities.has_initrd {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "has live filesystem: {}",
        if plan.capabilities.has_live_filesystem {
            "yes"
        } else {
            "no"
        }
    );

    if !plan.notes.is_empty() {
        println!("notes:");
        for note in &plan.notes {
            println!("  - {note}");
        }
    }

    if !plan.warnings.is_empty() {
        println!("warnings:");
        for warning in &plan.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_mode_selection(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    chunk_size: u64,
    output: OutputFormat,
) {
    match select_write_mode(image, target, confirmation_phrase, chunk_size) {
        Ok(selection) => {
            let ready = selection.status == WriteModeSelectionStatus::Selected;
            match output {
                OutputFormat::Text => print_mode_selection_report(selection),
                OutputFormat::Json => print!("{}", selection.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not select write mode.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_mode_selection_report(selection: WriteModeSelection) {
    println!("OSflux write mode selection");
    println!("status: {}", selection.status.as_str());
    println!("reason: {}", selection.reason);
    println!(
        "selected mode: {}",
        selection
            .selected_mode
            .map(|mode| mode.as_str().to_string())
            .unwrap_or_else(|| "unavailable".to_string())
    );
    println!("image: {}", selection.image_source);
    println!("image kind: {}", selection.image_kind);
    println!("image status: {}", selection.image_status.as_str());
    println!(
        "image strategy: {}",
        selection.image_write_strategy.as_str()
    );
    println!("session id: {}", selection.session_id);
    println!("target index: {}", selection.target_index);
    println!("target path: {}", selection.target_path);
    println!("chunk count: {}", selection.chunk_count);
    println!("planned bytes: {}", format_size(selection.planned_bytes));
    println!("dry run: {}", if selection.dry_run { "yes" } else { "no" });
    println!(
        "write access attempted: {}",
        if selection.write_access_attempted {
            "yes"
        } else {
            "no"
        }
    );

    if !selection.notes.is_empty() {
        println!("notes:");
        for note in selection.notes {
            println!("  - {note}");
        }
    }

    if !selection.warnings.is_empty() {
        println!("warnings:");
        for warning in selection.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_progress_preview(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    chunk_size: u64,
    output: OutputFormat,
) {
    match preview_write_progress(image, target, confirmation_phrase, chunk_size) {
        Ok(preview) => {
            let ready = preview.status == ProgressPreviewStatus::Ready;
            match output {
                OutputFormat::Text => print_progress_preview_report(preview),
                OutputFormat::Json => print!("{}", preview.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not prepare progress preview.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_progress_preview_report(preview: ProgressPreview) {
    println!("OSflux progress preview");
    println!("status: {}", preview.status.as_str());
    println!("reason: {}", preview.reason);
    println!("session id: {}", preview.session_id);
    println!("chunk plan status: {}", preview.chunk_plan_status.as_str());
    println!("total chunks: {}", preview.total_chunks);
    println!("total bytes: {}", format_size(preview.total_bytes));
    println!("event count: {}", preview.event_count);
    println!("dry run: {}", if preview.dry_run { "yes" } else { "no" });
    println!(
        "write access attempted: {}",
        if preview.write_access_attempted {
            "yes"
        } else {
            "no"
        }
    );

    if !preview.preview_events.is_empty() {
        println!("preview events:");
        for event in preview.preview_events {
            println!(
                "  - {}: {} chunk(s), {}, {}",
                event.label,
                event.completed_chunks,
                format_size(event.completed_bytes),
                format_basis_points(event.percent_complete_basis_points)
            );
        }
    }

    if !preview.notes.is_empty() {
        println!("notes:");
        for note in preview.notes {
            println!("  - {note}");
        }
    }

    if !preview.warnings.is_empty() {
        println!("warnings:");
        for warning in preview.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_chunk_plan(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    chunk_size: u64,
    output: OutputFormat,
) {
    match plan_write_chunks(image, target, confirmation_phrase, chunk_size) {
        Ok(plan) => {
            let ready = plan.status == ChunkPlanStatus::Planned;
            match output {
                OutputFormat::Text => print_chunk_plan_report(plan),
                OutputFormat::Json => print!("{}", plan.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not prepare chunk plan.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_chunk_plan_report(plan: ChunkPlan) {
    println!("OSflux chunk plan");
    println!("status: {}", plan.status.as_str());
    println!("reason: {}", plan.reason);
    println!("session id: {}", plan.session_id);
    println!("session status: {}", plan.session_status.as_str());
    println!("image: {}", plan.image_source);
    println!("target index: {}", plan.target_index);
    println!("target path: {}", plan.target_path);
    println!("image size: {}", format_size(plan.image_size_bytes));
    println!("chunk size: {}", format_size(plan.chunk_size_bytes));
    println!("chunk count: {}", plan.chunk_count);
    println!(
        "final chunk size: {}",
        format_size(plan.final_chunk_size_bytes)
    );
    println!("planned bytes: {}", format_size(plan.planned_bytes));
    println!("dry run: {}", if plan.dry_run { "yes" } else { "no" });
    println!(
        "write access attempted: {}",
        if plan.write_access_attempted {
            "yes"
        } else {
            "no"
        }
    );

    if !plan.notes.is_empty() {
        println!("notes:");
        for note in plan.notes {
            println!("  - {note}");
        }
    }

    if !plan.warnings.is_empty() {
        println!("warnings:");
        for warning in plan.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_write_session(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    output: OutputFormat,
) {
    match prepare_write_session(image, target, confirmation_phrase) {
        Ok(session) => {
            let ready = session.status == WriteSessionStatus::Prepared;
            match output {
                OutputFormat::Text => print_write_session_report(session),
                OutputFormat::Json => print!("{}", session.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not prepare write session.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_write_session_report(session: WriteSession) {
    println!("OSflux write session");
    println!("session id: {}", session.session_id);
    println!("status: {}", session.status.as_str());
    println!("reason: {}", session.reason);
    println!(
        "created at unix seconds: {}",
        session.created_at_unix_seconds
    );
    println!("image: {}", session.image_source);
    println!("target index: {}", session.target_index);
    println!("target path: {}", session.target_path);
    println!(
        "target identity code: {}",
        session.target_identity_code.as_deref().unwrap_or("unknown")
    );
    println!("command status: {}", session.command_status.as_str());
    println!(
        "confirmed: {}",
        if session.confirmed { "yes" } else { "no" }
    );
    println!("locked: {}", if session.locked { "yes" } else { "no" });
    println!("dry run: {}", if session.dry_run { "yes" } else { "no" });
    println!(
        "write access attempted: {}",
        if session.write_access_attempted {
            "yes"
        } else {
            "no"
        }
    );

    if !session.notes.is_empty() {
        println!("notes:");
        for note in session.notes {
            println!("  - {note}");
        }
    }

    if !session.warnings.is_empty() {
        println!("warnings:");
        for warning in session.warnings {
            println!("  - {warning}");
        }
    }
}

fn run_raw_write(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    chunk_size: u64,
    assume_yes: bool,
    output: OutputFormat,
) {
    let Some(confirmation_phrase) = confirmation_phrase else {
        eprintln!("--write --execute needs --confirm with the phrase printed by --intent");
        std::process::exit(2);
    };

    // Checked before any slow work: without a terminal there is nobody to
    // answer the final prompt, so only an explicit --yes may stand in for it.
    if !assume_yes && !std::io::stdin().is_terminal() {
        eprintln!("OSflux did not write the image.");
        eprintln!(
            "A final confirmation is needed, but there is no terminal to ask in. Pass --yes to confirm without a prompt."
        );
        std::process::exit(2);
    }

    let authorizations = open_authorization_store();
    let audit = open_audit_log();
    let plan = match prepare_raw_write(
        image,
        target,
        confirmation_phrase,
        chunk_size,
        &authorizations,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            record_audit_event(
                &audit,
                AuditEvent::new(AuditEventKind::WriteRefused, image, target)
                    .with_detail(error.to_string()),
            );
            exit_with_raw_write_error(error)
        }
    };

    if let OutputFormat::Text = output {
        print_raw_write_plan(&plan);
    }

    if !assume_yes {
        confirm_erase_interactively(&plan, &audit);
    }

    if let OutputFormat::Text = output {
        println!("Erasing the target and writing the image. Do not unplug the drive.");
    }

    let mut progress = RawWriteProgressPrinter::new(output);
    let result = write_image_to_target(&plan, &authorizations, &audit, |event| {
        progress.update(event)
    });
    progress.finish();

    match result {
        Ok(report) => {
            match output {
                OutputFormat::Text => print_raw_write_report(&report, &audit),
                OutputFormat::Json => print!("{}", report.to_json_pretty()),
            }
            if !report.audit_recorded {
                eprintln!(
                    "warning: the write succeeded, but its outcome could not be added to the audit log {}",
                    audit.path().display()
                );
            }
        }
        Err(error) => exit_with_raw_write_error(error),
    }
}

fn open_audit_log() -> AuditLog {
    match AuditLog::default_location() {
        Some(log) => log,
        None => {
            eprintln!("OSflux could not find where to keep its audit log.");
            eprintln!(
                "Set the OSFLUX_STATE_DIR environment variable to a folder OSflux can write to."
            );
            std::process::exit(1);
        }
    }
}

/// Adds an event that does not gate anything, warning instead of failing
/// when the log cannot be written.
fn record_audit_event(audit: &AuditLog, event: AuditEvent) {
    if let Err(error) = audit.record(&event) {
        eprintln!(
            "warning: could not add a {} event to the audit log {}: {error}",
            event.kind.as_str(),
            audit.path().display()
        );
    }
}

fn open_authorization_store() -> AuthorizationStore {
    match AuthorizationStore::default_location() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("OSflux could not find where to keep write confirmations.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_raw_write_plan(plan: &RawWritePlan) {
    println!("OSflux raw write");
    println!("session id: {}", plan.session_id);
    println!(
        "image: {} ({})",
        plan.image_path.display(),
        format_size(plan.image_size_bytes)
    );
    println!("target index: {}", plan.target_index);
    println!("target path: {}", plan.target_path);
    println!(
        "target model: {}",
        plan.target_model.as_deref().unwrap_or("unknown")
    );
    println!("target size: {}", format_size(plan.target_size_bytes));
    println!("target identity code: {}", plan.target_identity_code);
}

/// Asks on the terminal before erasing; anything but "yes" cancels and
/// leaves the confirmation from --intent usable until it expires.
fn confirm_erase_interactively(plan: &RawWritePlan, audit: &AuditLog) {
    eprintln!();
    eprintln!(
        "This erases everything on {} ({}, {}).",
        plan.target_model.as_deref().unwrap_or("the target drive"),
        format_size(plan.target_size_bytes),
        plan.target_path
    );
    eprint!("Type yes to continue: ");
    let _ = std::io::stderr().flush();

    let mut answer = String::new();
    let answered = std::io::stdin().read_line(&mut answer).is_ok();
    if !answered || !is_erase_confirmation(&answer) {
        record_audit_event(
            audit,
            AuditEvent::for_plan(AuditEventKind::WriteCancelled, plan),
        );
        eprintln!(
            "Cancelled. Nothing was written. The confirmation stays usable until {}.",
            format_unix_utc(plan.authorization_expires_at_unix_seconds)
        );
        let _ = std::io::stdout().flush();
        std::process::exit(EXIT_NOT_READY);
    }
}

fn is_erase_confirmation(answer: &str) -> bool {
    answer.trim().eq_ignore_ascii_case("yes")
}

fn print_raw_write_report(report: &RawWriteReport, audit: &AuditLog) {
    println!("status: written");
    println!("image sha256: {}", report.image_sha256);
    println!(
        "wrote {} in {}",
        format_size(report.bytes_written),
        format_rate(report.bytes_written, report.write_millis)
    );
    println!(
        "verified {} in {}",
        format_size(report.bytes_verified),
        format_rate(report.bytes_verified, report.verify_millis)
    );
    println!("audit log: {}", audit.path().display());
    println!("The drive can be removed.");
}

fn format_rate(bytes: u64, millis: u64) -> String {
    let seconds = millis as f64 / 1000.0;
    if seconds <= 0.0 {
        return format!("{seconds:.1} s");
    }

    let mib_per_second = bytes as f64 / (1024.0 * 1024.0) / seconds;
    format!("{seconds:.1} s ({mib_per_second:.1} MiB/s)")
}

fn exit_with_raw_write_error(error: RawWriteError) -> ! {
    let exit_code = match error {
        RawWriteError::InvalidChunkSize { .. } => 2,
        RawWriteError::Blocked { .. }
        | RawWriteError::ConfirmationMismatch { .. }
        | RawWriteError::Authorization { .. }
        | RawWriteError::TargetChanged { .. } => EXIT_NOT_READY,
        _ => 1,
    };

    eprintln!("OSflux did not write the image.");
    eprintln!("{error}");
    match error {
        RawWriteError::Open(_) => eprintln!(
            "Run PowerShell as Administrator, and close any programs or windows using the drive."
        ),
        RawWriteError::Device(_) | RawWriteError::VerifyMismatch { .. } => eprintln!(
            "The drive may now hold a partial or damaged image. Write it again, or try another drive."
        ),
        _ => {}
    }

    let _ = std::io::stdout().flush();
    std::process::exit(exit_code);
}

/// Shows progress on one line, redrawn only when the whole percentage changes.
struct RawWriteProgressPrinter {
    enabled: bool,
    last: Option<(RawWritePhase, u64)>,
}

impl RawWriteProgressPrinter {
    fn new(output: OutputFormat) -> Self {
        Self {
            enabled: matches!(output, OutputFormat::Text),
            last: None,
        }
    }

    fn update(&mut self, event: RawWriteProgress) {
        if !self.enabled {
            return;
        }

        let percent = (event.completed_bytes * 100)
            .checked_div(event.total_bytes)
            .unwrap_or(100);
        match self.last {
            Some((phase, last_percent)) if phase == event.phase && last_percent == percent => {
                return
            }
            Some((phase, _)) if phase != event.phase => eprintln!(),
            _ => {}
        }

        eprint!(
            "\r{}: {percent:>3}% ({} of {})",
            event.phase.as_str(),
            format_size(event.completed_bytes),
            format_size(event.total_bytes)
        );
        let _ = std::io::stderr().flush();
        self.last = Some((event.phase, percent));
    }

    fn finish(&mut self) {
        if self.enabled && self.last.is_some() {
            eprintln!();
        }
    }
}

fn print_write_command(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    output: OutputFormat,
) {
    match prepare_write_command(image, target, confirmation_phrase) {
        Ok(plan) => {
            let ready = plan.status == WriteCommandStatus::ReadyDryRun;
            match output {
                OutputFormat::Text => print_write_command_plan(plan),
                OutputFormat::Json => print!("{}", plan.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not plan write command.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_write_command_plan(plan: WriteCommandPlan) {
    println!("OSflux write command");
    println!("status: {}", plan.status.as_str());
    println!("reason: {}", plan.reason);
    println!("image: {}", plan.image_source);
    println!("target index: {}", plan.target_index);
    println!("target path: {}", plan.target_path);
    println!(
        "target identity code: {}",
        plan.target_identity_code.as_deref().unwrap_or("unknown")
    );
    println!("intent status: {}", plan.intent_status.as_str());
    println!(
        "supplied confirmation: {}",
        if plan.supplied_confirmation_phrase_present {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "confirmation matches: {}",
        if plan.confirmation_matches {
            "yes"
        } else {
            "no"
        }
    );
    println!("dry run: {}", if plan.dry_run { "yes" } else { "no" });
    println!(
        "write access attempted: {}",
        if plan.write_access_attempted {
            "yes"
        } else {
            "no"
        }
    );

    if let Some(expected_confirmation_phrase) = plan.expected_confirmation_phrase {
        println!("expected confirmation phrase: {expected_confirmation_phrase}");
    } else {
        println!("expected confirmation phrase: unavailable");
    }

    if !plan.notes.is_empty() {
        println!("notes:");
        for note in plan.notes {
            println!("  - {note}");
        }
    }

    if !plan.warnings.is_empty() {
        println!("warnings:");
        for warning in plan.warnings {
            println!("  - {warning}");
        }
    }
}

fn parse_target_index(value: &str) -> u32 {
    match value.parse::<u32>() {
        Ok(index) => index,
        Err(_) => {
            eprintln!("target index must be a non-negative number");
            std::process::exit(2);
        }
    }
}

fn parse_u64_option(value: &str, name: &str) -> u64 {
    match value.parse::<u64>() {
        Ok(parsed) => parsed,
        Err(_) => {
            eprintln!("{name} must be a non-negative number");
            std::process::exit(2);
        }
    }
}

fn parse_usize_option(value: &str, name: &str) -> usize {
    match value.parse::<usize>() {
        Ok(parsed) => parsed,
        Err(_) => {
            eprintln!("{name} must be a non-negative number");
            std::process::exit(2);
        }
    }
}

fn exit_unless_ready(ready: bool) {
    if !ready {
        let _ = std::io::stdout().flush();
        std::process::exit(EXIT_NOT_READY);
    }
}

fn print_write_intent(image: &str, target: u32, output: OutputFormat) {
    match create_write_intent(image, target) {
        Ok(mut intent) => {
            let ready = intent.status == WriteIntentStatus::ConfirmationRequired;
            if ready {
                let authorizations = open_authorization_store();
                if let Err(error) = record_write_authorization(
                    &mut intent,
                    image,
                    &authorizations,
                    now_unix_seconds(),
                ) {
                    eprintln!("OSflux could not record the confirmation.");
                    eprintln!("{error}");
                    std::process::exit(1);
                }
                if let Some(audit) = AuditLog::default_location() {
                    record_audit_event(
                        &audit,
                        AuditEvent::for_intent(AuditEventKind::Authorized, &intent),
                    );
                }
            }

            match output {
                OutputFormat::Text => print_write_intent_report(intent),
                OutputFormat::Json => print!("{}", intent.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not create write intent.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_write_intent_report(intent: WriteIntent) {
    println!("OSflux write intent");
    println!("status: {}", intent.status.as_str());
    println!("reason: {}", intent.reason);
    println!("image: {}", intent.image_source);
    println!("target index: {}", intent.target_index);
    println!("target path: {}", intent.target_path);
    println!(
        "target model: {}",
        intent.target_model.as_deref().unwrap_or("unknown")
    );
    println!(
        "target size: {}",
        intent
            .target_size_bytes
            .map(format_size)
            .unwrap_or_else(|| "size unknown".to_string())
    );
    println!(
        "target identity code: {}",
        intent.target_identity_code.as_deref().unwrap_or("unknown")
    );
    println!("preflight status: {}", intent.preflight_status.as_str());
    println!("preflight reason: {}", intent.preflight_reason);

    if let Some(confirmation_phrase) = intent.confirmation_phrase {
        println!("confirmation phrase: {confirmation_phrase}");
    } else {
        println!("confirmation phrase: unavailable");
    }

    if let Some(expires_at) = intent.authorization_expires_at_unix_seconds {
        println!(
            "confirmation valid: for one write, until {}",
            format_unix_utc(expires_at)
        );
    }

    if !intent.notes.is_empty() {
        println!("notes:");
        for note in intent.notes {
            println!("  - {note}");
        }
    }

    if !intent.warnings.is_empty() {
        println!("warnings:");
        for warning in intent.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_preflight(image: &str, target: u32, output: OutputFormat) {
    match run_write_preflight(image, target) {
        Ok(preflight) => {
            let ready = matches!(
                preflight.status,
                WritePreflightStatus::Ready | WritePreflightStatus::NeedsConfirmation
            );
            match output {
                OutputFormat::Text => print_preflight_report(preflight),
                OutputFormat::Json => print!("{}", preflight.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not complete write preflight.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_preflight_report(preflight: WritePreflight) {
    println!("OSflux write preflight");
    println!("status: {}", preflight.status.as_str());
    println!("reason: {}", preflight.reason);
    println!("image: {}", preflight.image_source);
    println!("image status: {}", preflight.image_status.as_str());
    println!("image reason: {}", preflight.image_status_reason);
    println!("image strategy: {}", preflight.image_write_strategy);
    println!("target index: {}", preflight.target_index);
    println!("target path: {}", preflight.target_path);
    println!(
        "target model: {}",
        preflight.target_model.as_deref().unwrap_or("unknown")
    );
    println!(
        "target size: {}",
        preflight
            .target_size_bytes
            .map(format_size)
            .unwrap_or_else(|| "size unknown".to_string())
    );
    println!(
        "target identity code: {}",
        preflight
            .target_identity_code
            .as_deref()
            .unwrap_or("unknown")
    );
    println!("target status: {}", preflight.target_status.as_str());
    println!("target reason: {}", preflight.target_reason);
    println!(
        "requires confirmation: {}",
        if preflight.requires_confirmation {
            "yes"
        } else {
            "no"
        }
    );

    if !preflight.notes.is_empty() {
        println!("notes:");
        for note in preflight.notes {
            println!("  - {note}");
        }
    }

    if !preflight.warnings.is_empty() {
        println!("warnings:");
        for warning in preflight.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_image_plan(path: &str, output: OutputFormat) {
    match plan_image(path) {
        Ok(plan) => {
            let report = build_image_plan_report(&plan);
            match output {
                OutputFormat::Text => print_report(report),
                OutputFormat::Json => print!("{}", report.to_json_pretty()),
            }
            exit_unless_ready(plan.status == ImagePlanStatus::Ready);
        }
        Err(error) => {
            eprintln!("OSflux could not plan this image.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_target_assessment(index: u32, output: OutputFormat) {
    match assess_target(index) {
        Ok(assessment) => {
            let ready = assessment.status != TargetAssessmentStatus::Blocked;
            match output {
                OutputFormat::Text => print_assessment(assessment),
                OutputFormat::Json => print!("{}", assessment.to_json_pretty()),
            }
            exit_unless_ready(ready);
        }
        Err(error) => {
            eprintln!("OSflux could not assess this target.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_assessment(assessment: TargetAssessment) {
    println!("OSflux target assessment");
    println!("index: {}", assessment.index);
    println!("path: {}", assessment.path);
    println!("status: {}", assessment.status.as_str());
    println!("reason: {}", assessment.reason);
    println!(
        "size: {}",
        assessment
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "size unknown".to_string())
    );
    println!(
        "identity code: {}",
        assessment.identity_code.as_deref().unwrap_or("unknown")
    );
    println!(
        "serial number: {}",
        assessment
            .serial_number
            .as_deref()
            .unwrap_or("not reported")
    );
    println!("access: {}", assessment.access.as_str());
    println!(
        "system volume: {}",
        if assessment.contains_system_volume {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "likely removable: {}",
        if assessment.likely_removable {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "requires confirmation: {}",
        if assessment.requires_confirmation {
            "yes"
        } else {
            "no"
        }
    );

    if let Some(model) = assessment.model {
        println!("model: {model}");
    }

    if let Some(interface_type) = assessment.interface_type {
        println!("interface: {interface_type}");
    }

    if let Some(media_type) = assessment.media_type {
        println!("media: {media_type}");
    }

    if !assessment.notes.is_empty() {
        println!("notes:");
        for note in assessment.notes {
            println!("  - {note}");
        }
    }

    if !assessment.warnings.is_empty() {
        println!("warnings:");
        for warning in assessment.warnings {
            println!("  - {warning}");
        }
    }
}

fn print_report(report: PlanReport) {
    println!("{}", report.title);

    for section in report.sections {
        println!("{}:", section.title);

        for field in section.fields {
            println!("  {}: {}", field.label, field.value);
        }

        for item in section.items {
            println!("  - {}", item.title);

            for field in item.fields {
                println!("      {}: {}", field.label, field.value);
            }
        }
    }
}

fn print_devices() {
    println!("OSflux device scan");

    let devices = list_devices();
    if devices.is_empty() {
        println!("No physical drives were visible to OSflux.");
        println!("Try running the terminal as Administrator if you expected to see drives.");
        return;
    }

    for device in devices {
        let size = device
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "size unknown".to_string());

        println!(
            "[{}] {} | {} | {} | {}",
            device.index,
            device.path,
            size,
            device.access.as_str(),
            device.safety_label()
        );

        if let Some(model) = &device.model {
            println!("    model: {model}");
        }

        if let Some(serial_number) = &device.serial_number {
            println!("    serial: {serial_number}");
        }

        if let Some(interface_type) = &device.interface_type {
            println!("    interface: {interface_type}");
        }

        if let Some(media_type) = &device.media_type {
            println!("    media: {media_type}");
        }

        if device.contains_system_volume {
            println!("    warning: contains the Windows system volume; do not write to this drive");
        } else if device.is_external_disk() {
            println!("    warning: external hard disk or SSD; OSflux will not write to this drive");
        } else if device.is_likely_removable_target() {
            println!("    note: removable media; possible write target");
        }

        if let Some(note) = device.note {
            println!("    {note}");
        }
    }
}

fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;

    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", bytes as f64 / GIB)
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

fn format_basis_points(basis_points: u32) -> String {
    format!("{}.{:02}%", basis_points / 100, basis_points % 100)
}

fn print_help() {
    println!("OSflux CLI");
    println!("Usage:");
    println!("  cargo run -- --list-devices");
    println!(
        "  cargo run -- --select-mode --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\""
    );
    println!(
        "  cargo run -- --select-mode --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\" --chunk-size 4194304 --json"
    );
    println!(
        "  cargo run -- --progress-preview --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\""
    );
    println!(
        "  cargo run -- --progress-preview --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\" --chunk-size 4194304 --json"
    );
    println!(
        "  cargo run -- --chunk-plan --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\""
    );
    println!(
        "  cargo run -- --chunk-plan --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\" --chunk-size 4194304 --json"
    );
    println!(
        "  cargo run -- --session --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\""
    );
    println!(
        "  cargo run -- --session --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\" --json"
    );
    println!(
        "  cargo run -- --write --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\""
    );
    println!(
        "  cargo run -- --write --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\" --json"
    );
    println!(
        "  cargo run -- --write --image <PATH.img> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX> <CODE>\" --execute"
    );
    println!("  cargo run -- --intent --image <PATH> --target <INDEX>");
    println!("  cargo run -- --intent --image <PATH> --target <INDEX> --json");
    println!("  cargo run -- --preflight --image <PATH> --target <INDEX>");
    println!("  cargo run -- --preflight --image <PATH> --target <INDEX> --json");
    println!("  cargo run -- --assess-target <INDEX>");
    println!("  cargo run -- --assess-target <INDEX> --json");
    println!("  cargo run -- --detect-family --image <PATH>");
    println!("  cargo run -- --detect-family --image <PATH> --json");
    println!("  cargo run -- --filesystem-plan --image <PATH>");
    println!("  cargo run -- --filesystem-plan --image <PATH> --json");
    println!("  cargo run -- --plan-image <PATH>");
    println!("  cargo run -- --plan-image <PATH> --json");
    println!("  cargo run -- --blocks=16 --block-size=1048576");
    println!();
    println!("Options:");
    println!("  --list-devices    List visible physical drives without writing to them.");
    println!("  --select-mode     Select the read-only write mode for the planned image.");
    println!("  --progress-preview  Prepare dry-run progress events from a chunk plan.");
    println!("  --chunk-plan      Prepare a dry-run chunk plan for progress scheduling.");
    println!("  --session         Prepare a locked dry-run write session.");
    println!("  --write           Validate a confirmed write command as a dry-run skeleton.");
    println!(
        "  --execute         With --write: erase the target and write a raw disk image, then verify it."
    );
    println!(
        "  --yes             With --write --execute: skip the final prompt, for scripts and automation."
    );
    println!("  --intent          Create a read-only write intent and confirmation phrase.");
    println!("  --preflight       Combine image and target checks without writing.");
    println!("  --detect-family   Detect image family markers without writing.");
    println!(
        "  --filesystem-plan  Combine ISO family and boot assets into a read-only filesystem plan."
    );
    println!(
        "  --image           Image path for --filesystem-plan, --detect-family, --select-mode, --progress-preview, --chunk-plan, --session, --write, --intent, or --preflight."
    );
    println!(
        "  --target          Physical drive index for --select-mode, --progress-preview, --chunk-plan, --session, --write, --intent, or --preflight."
    );
    println!(
        "  --confirm         Exact confirmation phrase for --select-mode, --progress-preview, --chunk-plan, --session, or --write."
    );
    println!(
        "  --chunk-size      Chunk size in bytes for --select-mode, --progress-preview, --chunk-plan, or --write --execute."
    );
    println!("  --assess-target   Assess a physical drive index without writing to it.");
    println!("  --plan-image      Inspect an ISO or disk image and show the planned mode.");
    println!("  --json            Print command output as JSON when supported.");
    println!("  --blocks=<N>        Number of blocks to enqueue in the demo pipeline.");
    println!("  --block-size=<N>    Bytes per block in the demo pipeline.");
    println!();
    println!("Exit codes:");
    println!("  0  The command ran and its result passed.");
    println!("  1  The command could not run, for example because the image could not be read.");
    println!("  2  An argument was unknown, missing, or invalid.");
    println!(
        "  3  The command ran, but its result is blocked or needs inspection, or the final prompt was declined."
    );
}

#[cfg(test)]
mod tests {
    use super::is_erase_confirmation;

    #[test]
    fn only_yes_confirms_an_erase() {
        for answer in ["yes", "YES\n", "  Yes \r\n"] {
            assert!(is_erase_confirmation(answer), "{answer:?}");
        }

        for answer in [
            "",
            "\n",
            "y",
            "no",
            "yes please",
            "OSFLUX ERASE TARGET 3 2E85",
        ] {
            assert!(!is_erase_confirmation(answer), "{answer:?}");
        }
    }
}
