use osflux_engine::{
    assess_target, create_write_intent, list_devices, plan_image_report, run_write_preflight,
    Engine, EngineConfig, PlanReport, TargetAssessment, WriteIntent, WritePreflight,
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
    let mut preflight_image = None;
    let mut preflight_target = None;

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

fn parse_target_index(value: &str) -> u32 {
    match value.parse::<u32>() {
        Ok(index) => index,
        Err(_) => {
            eprintln!("target index must be a non-negative number");
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

fn print_help() {
    println!("OSflux CLI");
    println!("Usage:");
    println!("  cargo run -- --list-devices");
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
    println!("  --intent          Create a read-only write intent and confirmation phrase.");
    println!("  --preflight       Combine image and target checks without writing.");
    println!("  --image           Image path for --intent or --preflight.");
    println!("  --target          Physical drive index for --intent or --preflight.");
    println!("  --assess-target   Assess a physical drive index without writing to it.");
    println!("  --plan-image      Inspect an ISO or disk image and show the planned mode.");
    println!("  --json            Print image plan output as JSON.");
    println!("  --blocks=<N>        Number of blocks to enqueue in the demo pipeline.");
    println!("  --block-size=<N>    Bytes per block in the demo pipeline.");
}
