use forge_shell::{Args, Config};
use structopt::StructOpt;

fn main() -> eyre::Result<()> {
    pretty_env_logger::init();

    if let Err(err) = run() {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
    Ok(())
}

// TODO move to forge cli later
fn run() -> eyre::Result<()> {
    let args = Args::from_args();
    let config = Config::default();

    forge_shell::run(args, config)
}
