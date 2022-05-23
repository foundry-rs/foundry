//! Entrypoint for main `foundryup` command

use crate::{cli::self_update, config::Config, utils::ExitCode};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[clap(
    name = "forge", version = crate::utils::VERSION_MESSAGE,
    after_help = "Find more information in the book: http://book.getfoundry.sh"
)]
pub struct Foundryup {
    #[clap(
        short,
        long,
        help = "Install a specific branch from https://github.com/foundry-rs/foundry"
    )]
    pub branch: Option<String>,

    #[clap(name = "self", subcommand)]
    pub self_cmd: Option<FoundryupSelf>,
}

#[derive(Debug, Subcommand)]
#[clap(about = "Build, test, foundryup installation.")]
pub enum FoundryupSelf {
    #[clap(about = "Download and install foundryup updates.")]
    Update,
    #[clap(about = "Uninstall foundryup.")]
    Uninstall,
}

/// Executes the `foundryup` command
pub async fn run() -> eyre::Result<ExitCode> {
    let cmd: Foundryup = Foundryup::parse();

    let config = Config::new()?;
    if let Some(ref self_cmd) = cmd.self_cmd {
        return match self_cmd {
            FoundryupSelf::Update => self_update::update(&config).await,
            FoundryupSelf::Uninstall => self_update::uninstall(),
        }
    }

    Ok(0.into())
}
