use crate::{
    cmd::{Cmd, LoadConfig},
    opts::Dependency,
    utils::CommandUtils,
};
use clap::{Parser, ValueHint};
use eyre::WrapErr;
use foundry_config::{find_git_root_path, impl_figment_convert_basic};
use std::{path::PathBuf, process::Command};

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
}
impl_figment_convert_basic!(RemoveArgs);

impl Cmd for RemoveArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let prj_root = config.__root.0.clone();
        let git_root =
            find_git_root_path(&prj_root).wrap_err("Unable to detect git root directory")?;
        let libs = config.install_lib_dir();
        let libs_relative = libs
            .strip_prefix(prj_root)
            .wrap_err("Dependencies are not relative to project root")?;
        let git_mod_libs = git_root.join(".git/modules").join(libs_relative);

        self.dependencies.iter().try_for_each(|dep| -> eyre::Result<_> {
            let target_dir: PathBuf =
                if let Some(alias) = &dep.alias { alias } else { &dep.name }.into();

            let mut git_mod_path = git_mod_libs.join(&target_dir);
            let mut dep_path = libs.join(&target_dir);
            // handle relative paths that start with the install dir, so we convert `lib/forge-std`
            // to `forge-std`
            if !dep_path.exists() {
                if let Ok(rel_target) = target_dir.strip_prefix(libs_relative) {
                    dep_path = libs.join(rel_target);
                    git_mod_path = git_mod_libs.join(rel_target);
                }
            }

            if !dep_path.exists() {
                eyre::bail!("{}: No such dependency", target_dir.display());
            }

            println!(
                "Removing {} in {dep_path:?}, (url: {:?}, tag: {:?})",
                dep.name, dep.url, dep.tag
            );

            // remove submodule entry from .git/config
            Command::new("git")
                .args(["submodule", "deinit", "-f", &dep_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;

            // remove the submodule repository from .git/modules directory
            Command::new("rm")
                .args(["-rf", &git_mod_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;

            // remove the leftover submodule directory
            Command::new("git")
                .args(["rm", "-f", &dep_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;

            Ok(())
        })
    }
}
