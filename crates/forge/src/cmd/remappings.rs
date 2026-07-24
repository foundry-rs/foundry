use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_config::impl_figment_convert_basic;
use std::path::PathBuf;

/// CLI arguments for `forge remappings`.
#[derive(Clone, Debug, Parser)]
pub struct RemappingArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
    /// Pretty-print the remappings, grouping each of them by context.
    #[arg(long)]
    pretty: bool,
}
impl_figment_convert_basic!(RemappingArgs);

impl RemappingArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;

        if self.pretty {
            let mut current_context = None;
            for remapping in &config.remappings {
                let context = remapping.context.as_deref();
                if current_context != Some(context) {
                    if let Some(context) = context {
                        sh_status!("Context: {context}")?;
                    } else {
                        sh_status!("Global:")?;
                    }
                    current_context = Some(context);
                }
                sh_println!("{remapping}")?;
            }
        } else {
            for remapping in config.remappings {
                sh_println!("{remapping}")?;
            }
        }

        Ok(())
    }
}
