use super::build::BuildArgs;
use clap::Parser;
use eyre::Result;
use foundry_cli::{
    json::{JsonMessage, print_json_success_with_warnings},
    opts::EvmArgs,
    utils::LoadConfig,
};
use foundry_common::shell;
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
            let warnings = fix_tomls();
            if shell::is_json() {
                let warnings = warnings
                    .into_iter()
                    .map(|warning| {
                        let details = serde_json::to_value(&warning).ok();
                        let message = warning.to_string();
                        let warning = JsonMessage::warning("config.fix", message);
                        if let Some(details) = details {
                            warning.with_details(details)
                        } else {
                            warning
                        }
                    })
                    .collect();
                print_json_success_with_warnings((), warnings)?;
            } else {
                for warning in warnings {
                    sh_warn!("{warning}")?;
                }
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
