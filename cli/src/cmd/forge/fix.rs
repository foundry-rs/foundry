//! fix command
use crate::cmd::utils::Cmd;
use clap::Parser;

/// Automatically fix stuff
#[derive(Debug, Clone, Parser)]
pub struct FixArgs {
    #[clap(help = "Migrate to next config foundry.toml layout", long)]
    config: bool,
}

impl Cmd for FixArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        todo!("convert and save if modified")
    }
}
