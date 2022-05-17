//! The main `foundryup` command-line interface

use foundryup::{cli, errors::FoundryupError, process::get_process, utils, utils::ExitCode};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    match foundryup().await {
        Err(err) => {
            eprintln!("{:?}", err);
            std::process::exit(1)
        }
        Ok(code) => std::process::exit(code.0),
    }
}

/// Runs `foundryup` and returns the error code
async fn foundryup() -> eyre::Result<ExitCode> {
    let process = get_process();
    // we rely on knowing where foundryup was executed from, so we check that we can
    // successfully get that location
    process.current_dir()?;
    utils::current_exe()?;

    match process.name().as_deref() {
        Some("foundryup") => {
            cli::foundryup::run().await;
        }
        Some(n) if n.starts_with("foundryup-setup") || n.starts_with("foundryup-init") => {}
        Some(_n) => {
            todo!()
        }
        None => return Err(FoundryupError::NoExeName.into()),
    }

    Ok(0.into())
}
