use clap::Parser;

fn main() {
    let cli = msfs2xp::cli::Cli::parse();
    msfs2xp::init_logging(cli.verbose);
    match msfs2xp::run(cli) {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(1);
        }
    }
}
