use clap::Parser;
use eyre::Result;
use foundry_common::shell;
use soldeer_commands::{Command, Verbosity};

/// Available subcommands for Soldeer, see <https://github.com/mario-eth/soldeer/blob/main/crates/commands/src/lib.rs>
/// for more information
#[derive(Clone, Debug, Parser)]
#[command(
    override_usage = "Native Solidity Package Manager, run `forge soldeer [COMMAND] --help` for more details"
)]
pub struct SoldeerArgs {
    /// Command must be one of the following init/install/login/push/uninstall/update/version.
    #[command(subcommand)]
    command: Command,
}

impl SoldeerArgs {
    pub async fn run(self) -> Result<()> {
        let verbosity = Verbosity::new(shell::verbosity(), if shell::is_quiet() { 1 } else { 0 });
        match soldeer_commands::run(self.command, verbosity).await {
            Ok(_) => Ok(()),
            Err(err) => Err(eyre::eyre!("Failed to run soldeer: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use soldeer_commands::{Command, Verbosity, commands::Version};

    #[tokio::test]
    async fn test_soldeer_version() {
        let command = Command::Version(Version::default());
        assert!(soldeer_commands::run(command, Verbosity::new(0, 1)).await.is_ok());
    }
}
