//! remappings command

use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use ethers::solc::{remappings::Remapping, ProjectPathsConfig};
use std::path::PathBuf;

/// Command to list remappings
#[derive(Debug, Clone, Parser)]
pub struct RemappingArgs {
    #[clap(
        help = "the project's root path, default being the current working directory",
        long,
        value_hint = ValueHint::DirPath
    )]
    root: Option<PathBuf>,
    #[clap(
        help = "the paths where your libraries are installed",
        long,
        value_hint = ValueHint::DirPath
    )]
    lib_paths: Vec<PathBuf>,
}

impl Cmd for RemappingArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = self.root.unwrap_or_else(|| std::env::current_dir().unwrap());
        let root = dunce::canonicalize(root)?;

        let lib_paths = if self.lib_paths.is_empty() {
            ProjectPathsConfig::find_libs(&root)
        } else {
            self.lib_paths
        };
        let remappings: Vec<_> = lib_paths.iter().flat_map(Remapping::find_many).collect();
        remappings.iter().for_each(|x| println!("{}", x));
        Ok(())
    }
}
