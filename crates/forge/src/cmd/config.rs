use std::path::PathBuf;

use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_common::shell;
use foundry_config::fix::fix_tomls;

/// CLI arguments for `forge config`.
#[derive(Clone, Debug, Parser)]
pub struct ConfigArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Print only a basic set of the currently set config values.
    #[arg(long)]
    basic: bool,

    /// Attempt to fix any configuration warnings.
    #[arg(long)]
    fix: bool,
}
foundry_config::impl_figment_convert_basic!(ConfigArgs);

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
