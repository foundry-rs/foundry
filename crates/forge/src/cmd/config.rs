use super::build::BuildArgs;
use clap::Parser;
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_common::{evm::EvmArgs, shell};
use foundry_config::fix::fix_tomls;

foundry_config::impl_figment_convert!(ConfigArgs, build, evm);

/// CLI arguments for `forge config`.
#[derive(Clone, Debug, Parser)]
pub struct ConfigArgs {
    /// Print only a basic set of the currently set config values.
    #[arg(long)]
    basic: bool,

    /// Attempt to fix any configuration warnings.
    #[arg(long)]
    fix: bool,

    // support nested build arguments
    #[command(flatten)]
    build: BuildArgs,

    #[command(flatten)]
    evm: EvmArgs,
}

impl ConfigArgs {
    pub fn run(self) -> Result<()> {
        if self.fix {
            for warning in fix_tomls() {
                sh_warn!("{warning}")?;
            }
            return Ok(());
        }

        let config = self
            .load_config_unsanitized()?
            .normalized_optimizer_settings()
            // we explicitly normalize the version, so mimic the behavior when invoking solc
            .normalized_evm_version();

        let s = if self.basic {
            let config = config.into_basic();
            if shell::is_json() {
                serde_json::to_string_pretty(&config)?
            } else {
                config.to_string_pretty()?
            }
        } else if shell::is_json() {
            serde_json::to_string_pretty(&config)?
        } else {
            config.to_string_pretty()?
        };

        sh_println!("{s}")?;
        Ok(())
    }
}
