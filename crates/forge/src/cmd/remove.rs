use crate::Lockfile;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{
    opts::Dependency,
    utils::{Git, LoadConfig},
};
use foundry_config::impl_figment_convert_basic;
use std::path::PathBuf;

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
        let config = self.load_config()?;
        let (root, mut paths, _) = super::update::dependencies_paths(&self.dependencies, &config)?;

        // Canonicalize paths to ensure consistent path separators for git submodule config lookup
        paths = paths
            .into_iter()
            .map(|path| {
                dunce::canonicalize(root.join(&path))
                    .ok()
                    .and_then(|canonical| {
                        canonical.strip_prefix(&root).ok().map(|p| p.to_path_buf())
                    })
                    .unwrap_or(path)
            })
            .collect();

        let git_modules = root.join(".git/modules");

        let git = Git::new(&root);
        let mut lockfile = Lockfile::new(&config.root).with_git(&git);
        let _synced = lockfile.sync(config.install_lib_dir())?;

        // remove all the dependencies by invoking `git rm` only once with all the paths
        git.rm(self.force, &paths)?;

        // remove all the dependencies from .git/modules
        for (Dependency { name, tag, .. }, path) in self.dependencies.iter().zip(&paths) {
            // Get the URL from git submodule config instead of using the parsed dependency URL
            let url = git.submodule_url(path).unwrap_or(None);
            sh_println!("Removing '{name}' in {}, (url: {url:?}, tag: {tag:?})", path.display())?;
            let _ = lockfile.remove(path);
            std::fs::remove_dir_all(git_modules.join(path))?;
        }

        lockfile.write()?;

        Ok(())
    }
}
