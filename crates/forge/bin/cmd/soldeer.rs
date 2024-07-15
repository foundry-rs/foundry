use clap::Parser;
use eyre::Result;

use soldeer::commands::Subcommands;

// CLI arguments for `forge soldeer`.
#[derive(Clone, Debug, Parser)]
#[clap(override_usage = "forge soldeer install [DEPENDENCY]~[VERSION] <REMOTE_URL>
    forge soldeer install [DEPENDENCY]~[VERSION] <GIT_URL>
    forge soldeer install [DEPENDENCY]~[VERSION] <GIT_URL> --rev <REVISION>
    forge soldeer install [DEPENDENCY]~[VERSION] <GIT_URL> --rev <TAG>
    forge soldeer push [DEPENDENCY]~[VERSION] <CUSTOM_PATH_OF_FILES>
    forge soldeer login
    forge soldeer update
    forge soldeer version")]
pub struct SoldeerArgs {
    /// Command must be one of the following install/push/login/update/version.
    #[command(subcommand)]
    command: Subcommands,
}

impl SoldeerArgs {
    pub fn run(self) -> Result<()> {
        match soldeer::run(self.command) {
            Ok(_) => Ok(()),
            Err(err) => Err(eyre::eyre!("Failed to run soldeer {}", err.message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soldeer::commands::VersionDryRun;

    #[test]
    fn test_soldeer_version() {
        assert!(soldeer::run(Subcommands::VersionDryRun(VersionDryRun {})).is_ok());
    }
}
