use osflux_engine::{Engine, EngineConfig};

fn main() {
    let config = parse_config(std::env::args().skip(1));

    println!("OSflux CLI");
    let engine = Engine::new(config);
    engine.start();
}

fn parse_config<I>(args: I) -> EngineConfig
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut config = EngineConfig::default();

    for arg in args {
        let arg = arg.as_ref();
        if arg == "--help" || arg == "-h" {
            print_help();
            std::process::exit(0);
        } else if let Some(value) = arg.strip_prefix("--blocks=") {
            if let Ok(parsed) = value.parse::<usize>() {
                config.block_count = parsed;
            }
        } else if let Some(value) = arg.strip_prefix("--block-size=") {
            if let Ok(parsed) = value.parse::<usize>() {
                config.block_size = parsed;
            }
        }
    }

    config
}

fn print_help() {
    println!("OSflux CLI");
    println!("Usage:");
    println!("  cargo run -- --blocks=16 --block-size=1048576");
    println!();
    println!("Options:");
    println!("  --blocks=<N>        Number of blocks to enqueue in the demo pipeline.");
    println!("  --block-size=<N>    Bytes per block in the demo pipeline.");
}
