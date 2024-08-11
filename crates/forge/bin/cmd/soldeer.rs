use clap::Parser;
use eyre::Result;

use soldeer::commands::Subcommands;

// CLI arguments for `forge soldeer`.
#[derive(Clone, Debug, Parser)]
#[clap(override_usage = "Native Solidity Package Manager, `run forge soldeer [COMMAND] --help` for more details")]
pub struct SoldeerArgs {
    /// Command must be one of the following install/push/login/update/version.
    #[command(subcommand)]
    command: Subcommands,
}

impl SoldeerArgs {
    pub fn run(self) -> Result<()> {
        match soldeer::run(self.command) {
            Ok(_) => Ok(()),
            Err(err) => Err(eyre::eyre!("Failed to run soldeer {}", err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soldeer::commands::Version;

    #[test]
    fn test_soldeer_version() {
        assert!(soldeer::run(Subcommands::Version(Version {})).is_ok());
    }
}
