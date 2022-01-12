//! config command

use crate::cmd::Cmd;
use clap::Parser;
use foundry_config::Config;

/// Command to list currently set config values
#[derive(Debug, Clone, Parser)]
pub struct ConfigArgs {
    #[clap(help = "prints currently set config values as json", long)]
    json: bool,
}

impl Cmd for ConfigArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let cwd = dunce::canonicalize(std::env::current_dir()?)?;
        let config = Config::from(Config::figment_with_root(cwd));

        let s = if self.json {
            serde_json::to_string_pretty(&config)?
        } else {
            config.to_string_pretty()?
        };
        println!("{}", s);
        Ok(())
    }
}
