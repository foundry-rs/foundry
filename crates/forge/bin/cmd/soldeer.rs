use clap::Parser;
use eyre::Result;

extern crate soldeer_lib;

use soldeer_lib::commands::{Args, Install, Login, Push, Subcommands, Update, VersionDryRun};
use yansi::Paint;

// CLI arguments for `forge soldeer`.
#[derive(Clone, Debug, Parser)]
#[clap(override_usage = "forge soldeer install [DEPENDENCY]~[VERSION] <REMOTE_URL>
    forge soldeer push [DEPENDENCY]~[VERSION] <CUSTOM_PATH_OF_FILES>
    forge soldeer login
    forge soldeer update
    forge soldeer version")]
pub struct SoldeerArgs {
    /// Command must be one of the following install/push/login/update/version.
    #[clap(required = true)]
    command: String,

    /// Only push and install can have arguments.
    #[clap(required = false)]
    args: Option<Vec<String>>,
}

impl SoldeerArgs {
    pub fn run(self) -> Result<()> {
        let args: Args;
        match self.command.as_str() {
            "install" => {
                let remote_url: Option<String>;
                match self.args.clone().unwrap().len() {
                    1 => {
                        remote_url = None;
                    }
                    2 => {
                        remote_url = Some(self.args.clone().unwrap().get(1).unwrap().to_string());
                    }
                    _ => {
                        panic!("Invalid number of arguments");
                    }
                }
                args = Args {
                    command: Subcommands::Install(Install {
                        dependency: self.args.clone().unwrap().get(0).unwrap().to_string(),
                        remote_url,
                    }),
                };
            }
            "update" => {
                args = Args { command: Subcommands::Update(Update {}) };
            }
            "login" => {
                args = Args { command: Subcommands::Login(Login {}) };
            }
            "push" => {
                let custom_path: Option<String>;
                match self.args.clone().unwrap().len() {
                    1 => {
                        custom_path = None;
                    }
                    2 => {
                        custom_path = Some(self.args.clone().unwrap().get(1).unwrap().to_string());
                    }
                    _ => {
                        panic!("Invalid number of arguments");
                    }
                }
                args = Args {
                    command: Subcommands::Push(Push {
                        dependency: self.args.clone().unwrap().get(0).unwrap().to_string(),
                        path: custom_path,
                    }),
                };
            }
            "version" => {
                args = Args { command: Subcommands::VersionDryRun(VersionDryRun {}) };
            }
            _ => {
                eprintln!("{}", Paint::red("Invalid soldeer command"));
                std::process::exit(1)
            }
        }

        match soldeer_lib::run(args) {
            Ok(_) => {}
            Err(err) => {
                eprintln!("{}", Paint::red(err.message))
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_soldeer_version() {
        let args: Args = Args { command: Subcommands::VersionDryRun(VersionDryRun {}) };
        assert_eq!(soldeer_lib::run(args).is_err(), false);
    }
}
