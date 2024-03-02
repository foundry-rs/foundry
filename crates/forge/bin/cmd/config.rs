use super::build::BuildArgs;
use clap::Parser;
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_common::{evm::EvmArgs, term::cli_warn};
use foundry_config::fix::fix_tomls;

foundry_config::impl_figment_convert!(ConfigArgs, opts, evm_opts);

/// CLI arguments for `forge config`.
#[derive(Clone, Debug, Parser)]
pub struct ConfigArgs {
    /// Print only a basic set of the currently set config values.
    #[arg(long)]
    basic: bool,

    /// Print currently set config values as JSON.
    #[arg(long)]
    json: bool,

    /// Attempt to fix any configuration warnings.
    #[arg(long)]
    fix: bool,

    // support nested build arguments
    #[command(flatten)]
    opts: BuildArgs,

    #[command(flatten)]
    evm_opts: EvmArgs,
}

impl ConfigArgs {
    pub fn run(self) -> Result<()> {
        if self.fix {
            for warning in fix_tomls() {
                cli_warn!("{warning}");
            }
            return Ok(())
        }

        let config = self
            .try_load_config_unsanitized_emit_warnings()?
            // we explicitly normalize the version, so mimic the behavior when invoking solc
            .normalized_evm_version();

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
