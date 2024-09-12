use clap::Parser;
use eyre::Result;

use soldeer::commands::Subcommands;

// CLI arguments for `forge soldeer`.
// The following list of commands and their actions:
//
// forge soldeer install: looks up the config file and install all the dependencies that are present
// there forge soldeer install package~version: looks up on https://soldeer.xyz and if the package>version is there then add to config+lockfile and install new dependency. Replaces existing entry if version is different.
// forge soldeer install package~version url: same behavior as install but instead of looking at https://soldeer.xyz it choses the URL, which can be git or custom zip url
// forge soldeer update: same behavior as install looks up the config file and install all the
// dependencies that are present there. This will change in the future forge soldeer login: logs in into https://soldeer.xyz account
// forge soldeer push package~version: pushes files to the central repository
// forge soldeer version: checks soldeer version
// forge soldeer init: initializes a new project with minimal dependency for foundry setup, install
// latest forge-std version forge soldeer uninstall dependency: uninstalls a dependency, removes
// artifacts and configs

#[derive(Clone, Debug, Parser)]
#[clap(
    override_usage = "Native Solidity Package Manager, `run forge soldeer [COMMAND] --help` for more details"
)]
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
