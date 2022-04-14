//! remappings command

use crate::cmd::Cmd;
use clap::{Parser, ValueHint};
use ethers::solc::{remappings::Remapping, ProjectPathsConfig};
use std::path::{Path, PathBuf};

/// Command to list remappings
#[derive(Debug, Clone, Parser)]
pub struct RemappingArgs {
    #[clap(
        help = "The project's root path. Defaults to the current working directory.",
        long,
        value_hint = ValueHint::DirPath
    )]
    root: Option<PathBuf>,
    #[clap(
        help = "The path to the library folder.",
        long,
        value_hint = ValueHint::DirPath
    )]
    lib_path: Vec<PathBuf>,
}

impl Cmd for RemappingArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let root = self.root.unwrap_or_else(|| std::env::current_dir().unwrap());
        let root = dunce::canonicalize(root)?;

        let lib_path = if self.lib_path.is_empty() {
            ProjectPathsConfig::find_libs(&root)
        } else {
            self.lib_path
        };
        let remappings: Vec<_> =
            lib_path.iter().flat_map(|lib| relative_remappings(lib, &root)).collect();
        remappings.iter().for_each(|x| println!("{}", x));
        Ok(())
    }
}

/// Returns all remappings found in the `lib` path relative to `root`
pub fn relative_remappings(lib: &Path, root: &Path) -> Vec<Remapping> {
    Remapping::find_many(lib)
        .into_iter()
        .map(|r| r.into_relative(root).to_relative_remapping())
        .map(Into::into)
        .collect()
}
