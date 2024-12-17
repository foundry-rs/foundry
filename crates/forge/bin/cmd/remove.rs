use alloy_primitives::map::HashMap;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::Dependency,
    utils::{Git, LoadConfig, TagType},
};
use foundry_common::fs;
use foundry_config::impl_figment_convert_basic;
use std::path::PathBuf;

use super::install::FOUNDRY_LOCK;

/// CLI arguments for `forge remove`.
#[derive(Clone, Debug, Parser)]
pub struct RemoveArgs {
    /// The dependencies you want to remove.
    #[arg(required = true)]
    dependencies: Vec<Dependency>,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,

    /// Override the up-to-date check.
    #[arg(short, long)]
    force: bool,
}
impl_figment_convert_basic!(RemoveArgs);

impl RemoveArgs {
    pub fn run(self) -> Result<()> {
        let config = self.try_load_config_emit_warnings()?;
        let (root, paths, _) = super::update::dependencies_paths(&self.dependencies, &config)?;
        let git_modules = root.join(".git/modules");

        let git = Git::new(&root);
        let (mut foundry_lock, _) = crate::cmd::install::read_and_sync_foundry_lock(&root, &git)?;

        // remove all the dependencies by invoking `git rm` only once with all the paths
        git.rm(self.force, &paths)?;

        // remove all the dependencies from .git/modules
        for (Dependency { name, url, tag, .. }, path) in self.dependencies.iter().zip(&paths) {
            sh_println!("Removing '{name}' in {}, (url: {url:?}, tag: {tag:?})", path.display())?;
            let _ = foundry_lock.remove(path);
            std::fs::remove_dir_all(git_modules.join(path))?;
        }

        fs::write_json_file(&config.root.join(FOUNDRY_LOCK), &foundry_lock)?;

        Ok(())
    }
}
