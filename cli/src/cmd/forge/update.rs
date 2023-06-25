//! Update command
use crate::{
    cmd::{Cmd, LoadConfig},
    opts::Dependency,
    utils::Git,
};
use clap::{Parser, ValueHint};
use eyre::{Context, Result};
use foundry_config::{impl_figment_convert_basic, Config};
use std::path::PathBuf;

/// CLI arguments for `forge update`.
#[derive(Debug, Clone, Parser)]
pub struct UpdateArgs {
    /// The dependencies you want to update.
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
impl_figment_convert_basic!(UpdateArgs);

impl Cmd for UpdateArgs {
    type Output = ();

    fn run(self) -> Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let (root, paths) = dependencies_paths(&self.dependencies, &config)?;
        Git::new(&root).submodule_update(self.force, true, paths)
    }
}

/// Returns `(root, paths)` where `root` is the root of the Git repository and `paths` are the
/// relative paths of the dependencies.
pub fn dependencies_paths(deps: &[Dependency], config: &Config) -> Result<(PathBuf, Vec<PathBuf>)> {
    let git_root = Git::root_of(&config.__root.0)?;
    let libs = config.install_lib_dir();

    let mut paths = Vec::with_capacity(deps.len());
    for dep in deps {
        let name = dep.name();
        let dep_path = libs.join(name);
        let rel_path = dep_path
            .strip_prefix(&git_root)
            .wrap_err("Library directory is not relative to the repository root")?;
        if !dep_path.exists() {
            eyre::bail!("Could not find dependency {name:?} in {}", dep_path.display());
        }
        paths.push(rel_path.to_owned());
    }
    Ok((git_root, paths))
}
