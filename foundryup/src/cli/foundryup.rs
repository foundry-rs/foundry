//! Entrypoint for main `foundryup` command

use crate::utils::ExitCode;
use clap::Parser;

#[derive(Debug, Parser)]
#[clap(name = "forge", version = crate::utils::VERSION_MESSAGE,
after_help = "Find more information in the book: http://book.getfoundry.sh"
)]
pub struct Foundryup {
    #[clap(
        short,
        long,
        help = "Install a specific branch from https://github.com/foundry-rs/foundry"
    )]
    pub branch: Option<String>,
}

/// Executes the `foundryup` command
pub async fn run() -> eyre::Result<ExitCode> {
    let _cmd = Foundryup::parse();

    Ok(0.into())
}
