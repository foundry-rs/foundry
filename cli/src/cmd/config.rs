//! config command

use crate::cmd::{build::BuildArgs, Cmd};
use clap::Parser;
use foundry_config::Config;

/// Command to list currently set config values
#[derive(Debug, Clone, Parser)]
pub struct ConfigArgs {
    #[clap(help = "prints currently set config values as json", long)]
    json: bool,
    #[clap(help = "prints basic set of currently set config values", long)]
    basic: bool,
    // support nested build arguments
    #[clap(flatten)]
    opts: BuildArgs,
}

impl Cmd for ConfigArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = Config::from(&self.opts);
        let s = if self.basic {
            let config = config.into_basic();
            if self.json {
                serde_json::to_string_pretty(&config)?
            } else {
                config.to_string_pretty()?
            }
        } else if self.json {
            serde_json::to_string_pretty(&config)?
        } else {
            config.to_string_pretty()?
        };

        println!("{}", s);
        Ok(())
    }
}
