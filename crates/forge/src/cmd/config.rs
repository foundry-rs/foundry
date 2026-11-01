use super::build::BuildArgs;
use clap::Parser;
use eyre::Result;
use foundry_cli::{opts::EvmArgs, utils::LoadConfig};
use foundry_common::shell;
use foundry_config::{fix::fix_tomls_with_root, origin::OriginResolver};

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

    /// Show where each value was set: a config file and table, an environment variable, a
    /// command-line flag, or the default.
    #[arg(long, conflicts_with = "fix")]
    show_origin: bool,

    // support nested build arguments
    #[command(flatten)]
    build: BuildArgs,

    #[command(flatten)]
    evm: EvmArgs,
}

impl ConfigArgs {
    pub fn run(self) -> Result<()> {
        if self.fix {
            let root = self.build.build.project_paths.try_project_root()?;
            for warning in fix_tomls_with_root(root) {
                sh_warn!("{warning}")?;
            }
            return Ok(());
        }

        let loaded = self.load_config_unsanitized()?;
        let config = loaded
            .clone()
            .normalized_optimizer_settings()
            // we explicitly normalize the version, so mimic the behavior when invoking solc
            .normalized_evm_version();

        let s = if self.show_origin {
            let (toml, loaded) = if self.basic {
                (config.into_basic().to_string_pretty()?, loaded.into_basic().to_string_pretty()?)
            } else {
                (config.to_string_pretty()?, loaded.to_string_pretty()?)
            };
            let figment = self.figment();
            let root = self.build.build.project_paths.project_root();
            let (annotated, origins) =
                OriginResolver::new(&figment, root).annotate(&toml, &loaded)?;
            if shell::is_json() { serde_json::to_string_pretty(&origins)? } else { annotated }
        } else if self.basic {
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
