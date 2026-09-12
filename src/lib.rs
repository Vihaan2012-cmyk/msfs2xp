//! Convert Microsoft Flight Simulator 2020/2024 airports into X-Plane 12 scenery.

pub mod bgl;
pub mod cli;
pub mod convert;
pub mod geo;
pub mod materials;
pub mod model;
pub mod model3d;
pub mod objects;
pub mod package;
pub mod pipeline;
pub mod texture;
pub mod xplane;

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
        cli::Cmd::Convert(args) => pipeline::run_convert(&args),
        cli::Cmd::List(args) => pipeline::run_list(&args),
        cli::Cmd::Validate(args) => pipeline::run_validate(&args),
        cli::Cmd::Preview(_) => {
            eprintln!("preview is not implemented yet");
            Ok(2)
        }
    }
}
