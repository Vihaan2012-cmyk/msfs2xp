//! Convert Microsoft Flight Simulator 2020/2024 airports into X-Plane 12 scenery.

pub mod bgl;
pub mod cli;
pub mod geo;

use anyhow::Context;

/// Set up logging from the `-v` count.
pub fn init_logging(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(format!("msfs2xp={level}")));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .try_init();
}

/// Dispatch a parsed command line. Returns the process exit code.
pub fn run(cli: cli::Cli) -> anyhow::Result<i32> {
    match cli.cmd {
        cli::Cmd::Inspect(args) => {
            let data = std::fs::read(&args.file).with_context(|| format!("reading {}", args.file.display()))?;
            let text = bgl::inspect::inspect(
                &data,
                args.icao.as_deref(),
                bgl::inspect::InspectOptions { hex: args.hex },
            );
            print!("{text}");
            Ok(0)
        }
        _ => {
            eprintln!("not implemented yet");
            Ok(2)
        }
    }
}
