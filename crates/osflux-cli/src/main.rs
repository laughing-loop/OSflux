use osflux_engine::{list_devices, plan_image, BootImageRange, Engine, EngineConfig, ImagePlan};

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
    match plan_image(path) {
        Ok(plan) => print_plan(plan),
        Err(error) => {
            eprintln!("OSflux could not plan this image.");
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn print_plan(plan: ImagePlan) {
    println!("OSflux image plan");
    println!("source: {}", plan.source.display());
    println!("file: {}", plan.file_name);
    println!("size: {}", format_size(plan.size_bytes));
    println!("kind: {}", plan.kind.as_str());
    println!("detection: {}", plan.detection_method.as_str());
    println!("write mode: {}", plan.write_mode.as_str());

    if !plan.signatures.is_empty() {
        println!("signatures:");
        for signature in plan.signatures {
            println!("  - {}", signature.as_str());
        }
    }

    if let Some(metadata) = plan.boot_metadata {
        println!("boot metadata:");
        println!(
            "  bootable: {}",
            if metadata.is_bootable() { "yes" } else { "no" }
        );
        println!("  boot style: {}", metadata.boot_style().as_str());

        if let Some(info) = metadata.el_torito {
            println!("  boot catalog: found");
            println!("  catalog lba: {}", info.catalog_lba);
            println!("  platform: {}", info.platform.as_str());
            println!(
                "  validation entry: {}",
                if info.validation_entry_valid {
                    "valid"
                } else {
                    "invalid"
                }
            );
            println!(
                "  default entry: {}",
                if info.default_entry_bootable {
                    "bootable"
                } else {
                    "not bootable"
                }
            );
            println!("  emulation: {}", info.emulation.as_str());
            println!("  load rba: {}", info.load_rba);
            println!("  sector count: {}", info.sector_count);
            println!("  boot entries:");
            for entry in info.boot_entries {
                println!(
                    "    - {} | {} | {} | {} | load rba {} | sectors {} | range {}",
                    entry.source.as_str(),
                    entry.platform.as_str(),
                    if entry.bootable {
                        "bootable"
                    } else {
                        "not bootable"
                    },
                    entry.emulation.as_str(),
                    entry.load_rba,
                    entry.sector_count,
                    format_boot_image_range(entry.image_range)
                );
            }
        } else {
            println!("  boot catalog: not found");
        }
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

fn format_boot_image_range(range: BootImageRange) -> String {
    let mut parts = vec![range.status.as_str().to_string()];

    if let Some(start_offset) = range.start_offset {
        parts.push(format!("offset {}", format_size(start_offset)));
    }

    if let Some(required_bytes) = range.required_bytes {
        parts.push(format!("required {}", format_size(required_bytes)));
    }

    if let Some(end_offset) = range.end_offset {
        parts.push(format!("end {}", format_size(end_offset)));
    }

    parts.join(", ")
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
