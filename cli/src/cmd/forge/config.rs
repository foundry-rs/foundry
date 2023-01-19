//! config command

use crate::cmd::{forge::build::BuildArgs, utils::Cmd, LoadConfig};
use clap::Parser;
use foundry_common::{evm::EvmArgs, term::cli_warn};
use foundry_config::fix::fix_tomls;

foundry_config::impl_figment_convert!(ConfigArgs, opts, evm_opts);

/// CLI arguments for `forge config`.
#[derive(Debug, Clone, Parser)]
pub struct ConfigArgs {
    #[clap(help = "Print only a basic set of the currently set config values.", long)]
    basic: bool,

    #[clap(help = "Print currently set config values as JSON.", long)]
    json: bool,

    #[clap(help = "Attempt to fix any configuration warnings.", long)]
    fix: bool,

    // support nested build arguments
    #[clap(flatten)]
    opts: BuildArgs,

    #[clap(flatten)]
    evm_opts: EvmArgs,
}

impl Cmd for ConfigArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        if self.fix {
            for warning in fix_tomls() {
                cli_warn!("{warning}");
            }
            return Ok(())
        }

        let config = self.try_load_config_unsanitized_emit_warnings()?;

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

        println!("{s}");
        Ok(())
    }
}
