//! remappings command

use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use foundry_config::{find_project_root_path, Config};
use std::path::PathBuf;

/// Command to list remappings
#[derive(Debug, Clone, Parser)]
pub struct RemappingArgs {
    #[clap(
        help = "The project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
    root: Option<PathBuf>,
}

impl Cmd for RemappingArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = self.root.unwrap_or_else(|| find_project_root_path().unwrap());
        let config = Config::load_with_root(root);
        config.remappings.iter().for_each(|x| println!("{x}"));
        Ok(())
    }
}
