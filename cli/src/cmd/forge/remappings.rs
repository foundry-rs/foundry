//! remappings command

use crate::cmd::{Cmd, LoadConfig};
use clap::{Parser, ValueHint};
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
}
impl_figment_convert_basic!(RemappingArgs);

impl Cmd for RemappingArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        config.remappings.iter().for_each(|x| println!("{x}"));
        Ok(())
    }
}
