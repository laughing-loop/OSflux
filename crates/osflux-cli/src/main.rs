use osflux_engine::{
    assess_target, create_write_intent, list_devices, plan_image_report, plan_write_chunks,
    prepare_write_command, prepare_write_session, preview_write_progress, run_write_preflight,
    select_write_mode, ChunkPlan, Engine, EngineConfig, PlanReport, ProgressPreview,
    TargetAssessment, WriteCommandPlan, WriteIntent, WriteModeSelection, WritePreflight,
    WriteSession, DEFAULT_CHUNK_SIZE_BYTES,
};

fn main() {
    match parse_command(std::env::args().skip(1)) {
        Command::Run(config) => {
            println!("OSflux CLI");
            let engine = Engine::new(config);
            engine.start();
        }
        Command::ListDevices => print_devices(),
        Command::PlanImage { path, output } => print_image_plan(&path, output),
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
            output,
        } => print_write_command(&image, target, confirm.as_deref(), output),
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
    let mut session_requested = false;
    let mut chunk_plan_requested = false;
    let mut progress_preview_requested = false;
    let mut mode_selection_requested = false;
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
        } else if arg == "--session" {
            session_requested = true;
        } else if arg == "--chunk-plan" {
            chunk_plan_requested = true;
        } else if arg == "--progress-preview" {
            progress_preview_requested = true;
        } else if arg == "--select-mode" {
            mode_selection_requested = true;
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
            if let Ok(parsed) = value.parse::<usize>() {
                config.block_count = parsed;
            }
        } else if let Some(value) = arg.strip_prefix("--block-size=") {
            if let Ok(parsed) = value.parse::<usize>() {
                config.block_size = parsed;
            }
        }

        index += 1;
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

    if let Some(path) = plan_image_path {
        return Command::PlanImage { path, output };
    }

    if let Some(index) = target_index {
        return Command::AssessTarget { index, output };
    }

    Command::Run(config)
}

fn print_mode_selection(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    chunk_size: u64,
    output: OutputFormat,
) {
    match select_write_mode(image, target, confirmation_phrase, chunk_size) {
        Ok(selection) => match output {
            OutputFormat::Text => print_mode_selection_report(selection),
            OutputFormat::Json => print!("{}", selection.to_json_pretty()),
        },
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
        Ok(preview) => match output {
            OutputFormat::Text => print_progress_preview_report(preview),
            OutputFormat::Json => print!("{}", preview.to_json_pretty()),
        },
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
        Ok(plan) => match output {
            OutputFormat::Text => print_chunk_plan_report(plan),
            OutputFormat::Json => print!("{}", plan.to_json_pretty()),
        },
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
        Ok(session) => match output {
            OutputFormat::Text => print_write_session_report(session),
            OutputFormat::Json => print!("{}", session.to_json_pretty()),
        },
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

fn print_write_command(
    image: &str,
    target: u32,
    confirmation_phrase: Option<&str>,
    output: OutputFormat,
) {
    match prepare_write_command(image, target, confirmation_phrase) {
        Ok(plan) => match output {
            OutputFormat::Text => print_write_command_plan(plan),
            OutputFormat::Json => print!("{}", plan.to_json_pretty()),
        },
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

fn print_write_intent(image: &str, target: u32, output: OutputFormat) {
    match create_write_intent(image, target) {
        Ok(intent) => match output {
            OutputFormat::Text => print_write_intent_report(intent),
            OutputFormat::Json => print!("{}", intent.to_json_pretty()),
        },
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
    println!("preflight status: {}", intent.preflight_status.as_str());
    println!("preflight reason: {}", intent.preflight_reason);

    if let Some(confirmation_phrase) = intent.confirmation_phrase {
        println!("confirmation phrase: {confirmation_phrase}");
    } else {
        println!("confirmation phrase: unavailable");
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
        Ok(preflight) => match output {
            OutputFormat::Text => print_preflight_report(preflight),
            OutputFormat::Json => print!("{}", preflight.to_json_pretty()),
        },
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
    match plan_image_report(path) {
        Ok(report) => match output {
            OutputFormat::Text => print_report(report),
            OutputFormat::Json => print!("{}", report.to_json_pretty()),
        },
        Err(error) => {
            eprintln!("OSflux could not plan this image.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_target_assessment(index: u32, output: OutputFormat) {
    match assess_target(index) {
        Ok(assessment) => match output {
            OutputFormat::Text => print_assessment(assessment),
            OutputFormat::Json => print!("{}", assessment.to_json_pretty()),
        },
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

        if let Some(interface_type) = &device.interface_type {
            println!("    interface: {interface_type}");
        }

        if let Some(media_type) = &device.media_type {
            println!("    media: {media_type}");
        }

        if device.contains_system_volume {
            println!("    warning: contains the Windows system volume; do not write to this drive");
        } else if device.is_likely_removable_target() {
            println!("    note: likely removable/external target candidate");
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
        "  cargo run -- --select-mode --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\""
    );
    println!(
        "  cargo run -- --select-mode --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\" --chunk-size 4194304 --json"
    );
    println!(
        "  cargo run -- --progress-preview --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\""
    );
    println!(
        "  cargo run -- --progress-preview --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\" --chunk-size 4194304 --json"
    );
    println!(
        "  cargo run -- --chunk-plan --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\""
    );
    println!(
        "  cargo run -- --chunk-plan --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\" --chunk-size 4194304 --json"
    );
    println!(
        "  cargo run -- --session --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\""
    );
    println!(
        "  cargo run -- --session --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\" --json"
    );
    println!(
        "  cargo run -- --write --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\""
    );
    println!(
        "  cargo run -- --write --image <PATH> --target <INDEX> --confirm \"OSFLUX ERASE TARGET <INDEX>\" --json"
    );
    println!("  cargo run -- --intent --image <PATH> --target <INDEX>");
    println!("  cargo run -- --intent --image <PATH> --target <INDEX> --json");
    println!("  cargo run -- --preflight --image <PATH> --target <INDEX>");
    println!("  cargo run -- --preflight --image <PATH> --target <INDEX> --json");
    println!("  cargo run -- --assess-target <INDEX>");
    println!("  cargo run -- --assess-target <INDEX> --json");
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
    println!("  --intent          Create a read-only write intent and confirmation phrase.");
    println!("  --preflight       Combine image and target checks without writing.");
    println!(
        "  --image           Image path for --select-mode, --progress-preview, --chunk-plan, --session, --write, --intent, or --preflight."
    );
    println!(
        "  --target          Physical drive index for --select-mode, --progress-preview, --chunk-plan, --session, --write, --intent, or --preflight."
    );
    println!(
        "  --confirm         Exact confirmation phrase for --select-mode, --progress-preview, --chunk-plan, --session, or --write."
    );
    println!("  --chunk-size      Chunk size in bytes for --select-mode, --progress-preview, or --chunk-plan.");
    println!("  --assess-target   Assess a physical drive index without writing to it.");
    println!("  --plan-image      Inspect an ISO or disk image and show the planned mode.");
    println!("  --json            Print image plan output as JSON.");
    println!("  --blocks=<N>        Number of blocks to enqueue in the demo pipeline.");
    println!("  --block-size=<N>    Bytes per block in the demo pipeline.");
}
