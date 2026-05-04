use osflux_engine::{list_devices, plan_image_report, Engine, EngineConfig, PlanReport};

fn main() {
    match parse_command(std::env::args().skip(1)) {
        Command::Run(config) => {
            println!("OSflux CLI");
            let engine = Engine::new(config);
            engine.start();
        }
        Command::ListDevices => print_devices(),
        Command::PlanImage(path) => print_image_plan(&path),
    }
}

enum Command {
    Run(EngineConfig),
    ListDevices,
    PlanImage(String),
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

    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--help" || arg == "-h" {
            print_help();
            std::process::exit(0);
        } else if arg == "--list-devices" {
            return Command::ListDevices;
        } else if let Some(path) = arg.strip_prefix("--plan-image=") {
            return Command::PlanImage(path.to_string());
        } else if arg == "--plan-image" {
            let Some(path) = args.get(index + 1) else {
                eprintln!("missing path after --plan-image");
                std::process::exit(2);
            };

            return Command::PlanImage(path.to_string());
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

    Command::Run(config)
}

fn print_image_plan(path: &str) {
    match plan_image_report(path) {
        Ok(report) => print_report(report),
        Err(error) => {
            eprintln!("OSflux could not plan this image.");
            eprintln!("{error}");
            std::process::exit(1);
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
    println!("  cargo run -- --plan-image <PATH>");
    println!("  cargo run -- --blocks=16 --block-size=1048576");
    println!();
    println!("Options:");
    println!("  --list-devices    List visible physical drives without writing to them.");
    println!("  --plan-image      Inspect an ISO or disk image and show the planned mode.");
    println!("  --blocks=<N>        Number of blocks to enqueue in the demo pipeline.");
    println!("  --block-size=<N>    Bytes per block in the demo pipeline.");
}
