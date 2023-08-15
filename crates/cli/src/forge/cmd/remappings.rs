use cast::HashMap;
use clap::{Parser, ValueHint};
use ethers::solc::remappings::RelativeRemapping;
use eyre::Result;
use foundry_cli::utils::LoadConfig;
use foundry_config::impl_figment_convert_basic;
use std::path::PathBuf;

/// CLI arguments for `forge remappings`.
#[derive(Debug, Clone, Parser)]
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
    // TODO: Do people use `forge remappings >> file`?
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;

        if self.pretty {
            let groups = config.remappings.into_iter().fold(
                HashMap::new(),
                |mut groups: HashMap<Option<String>, Vec<RelativeRemapping>>, remapping| {
                    groups.entry(remapping.context.clone()).or_default().push(remapping);
                    groups
                },
            );
            for (group, remappings) in groups.into_iter() {
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
