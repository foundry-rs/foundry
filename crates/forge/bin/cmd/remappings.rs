use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_config::impl_figment_convert_basic;
use foundry_evm::hashbrown::HashMap;
use std::path::PathBuf;

/// CLI arguments for `forge remappings`.
#[derive(Clone, Debug, Parser)]
pub struct RemappingArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
    /// Pretty-print the remappings, grouping each of them by context.
    #[clap(long)]
    pretty: bool,
}
impl_figment_convert_basic!(RemappingArgs);

impl RemappingArgs {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;

        if self.pretty {
            let mut groups = HashMap::<_, Vec<_>>::with_capacity(config.remappings.len());
            for remapping in config.remappings {
                groups.entry(remapping.context.clone()).or_default().push(remapping);
            }
            for (group, remappings) in groups {
                if let Some(group) = group {
                    println!("Context: {group}");
                } else {
                    println!("Global:");
                }

                for mut remapping in remappings.into_iter() {
                    remapping.context = None; // avoid writing context twice
                    println!("- {remapping}");
                }
                println!();
            }
        } else {
            for remapping in config.remappings.into_iter() {
                println!("{remapping}");
            }
        }

        Ok(())
    }
}
