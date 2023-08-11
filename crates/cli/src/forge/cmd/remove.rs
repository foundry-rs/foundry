use clap::{Parser, ValueHint};
use foundry_cli::{
    opts::Dependency,
    utils::{Cmd, Git, LoadConfig},
};
use foundry_config::impl_figment_convert_basic;
use std::path::PathBuf;

/// CLI arguments for `forge remove`.
#[derive(Debug, Clone, Parser)]
pub struct RemoveArgs {
    /// The dependencies you want to remove.
    dependencies: Vec<Dependency>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[clap(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Override the up-to-date check.
    #[clap(short, long)]
    force: bool,
}
impl_figment_convert_basic!(RemoveArgs);

impl Cmd for RemoveArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let (root, paths) = super::update::dependencies_paths(&self.dependencies, &config)?;
        let git_modules = root.join(".git/modules");

        // remove all the dependencies by invoking `git rm` only once with all the paths
        Git::new(&root).rm(self.force, &paths)?;

        // remove all the dependencies from .git/modules
        for (Dependency { name, url, tag, .. }, path) in self.dependencies.iter().zip(&paths) {
            println!("Removing '{name}' in {}, (url: {url:?}, tag: {tag:?})", path.display());
            std::fs::remove_dir_all(git_modules.join(path))?;
        }

        Ok(())
    }
}
