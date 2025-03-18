use clap::Parser;
use eyre::Result;
use soldeer_commands::Command;

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
#[command(
    override_usage = "Native Solidity Package Manager, `run forge soldeer [COMMAND] --help` for more details"
)]
pub struct SoldeerArgs {
    /// Command must be one of the following init/install/login/push/uninstall/update/version.
    #[command(subcommand)]
    command: Command,
}

impl SoldeerArgs {
    pub async fn run(self) -> Result<()> {
        match soldeer_commands::run(self.command).await {
            Ok(_) => Ok(()),
            Err(err) => Err(eyre::eyre!("Failed to run soldeer {}", err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use soldeer_commands::{commands::Version, Command};

    #[tokio::test]
    async fn test_soldeer_version() {
        let command = Command::Version(Version::default());
        assert!(soldeer_commands::run(command).await.is_ok());
    }
}
