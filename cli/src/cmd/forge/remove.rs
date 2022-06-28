use crate::{opts::forge::Dependency, utils::CommandUtils, Cmd};
use clap::Parser;
use eyre::WrapErr;
use foundry_config::{find_git_root_path, find_project_root_path, Config};
use std::{path::PathBuf, process::Command};

/// Command to remove dependencies
#[derive(Debug, Clone, Parser)]
pub struct RemoveArgs {
    #[clap(help = "The path to the dependency you want to remove.")]
    dependencies: Vec<Dependency>,
}

impl Cmd for RemoveArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let git_root = find_git_root_path().wrap_err("Unable to detect git root directory")?;
        let project_root = find_project_root_path().unwrap();

        let config = Config::load_with_root(&project_root);
        let install_lib_dir = config.install_lib_dir();

        let libs = project_root.join(&install_lib_dir);
        let git_mod_libs = git_root.join(".git/modules").join(&install_lib_dir);

        self.dependencies.iter().try_for_each(|dep| -> eyre::Result<_> {
            let target_dir: PathBuf =
                if let Some(alias) = &dep.alias { alias } else { &dep.name }.into();

            let mut git_mod_path = git_mod_libs.join(&target_dir);
            let mut dep_path = libs.join(&target_dir);
            // handle relative paths that start with the install dir, so we convert `lib/forge-std`
            // to `forge-std`
            if !dep_path.exists() {
                if let Ok(rel_target) = target_dir.strip_prefix(&install_lib_dir) {
                    dep_path = libs.join(&rel_target);
                    git_mod_path = git_mod_libs.join(&rel_target);
                }
            }

            if !dep_path.exists() {
                eyre::bail!("{}: No such dependency", target_dir.display());
            }

            println!(
                "Removing {} in {:?}, (url: {:?}, tag: {:?})",
                dep.name, dep_path, dep.url, dep.tag
            );

            // remove submodule entry from .git/config
            Command::new("git")
                .args(&["submodule", "deinit", "-f", &dep_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;

            // remove the submodule repository from .git/modules directory
            Command::new("rm")
                .args(&["-rf", &git_mod_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;

            // remove the leftover submodule directory
            Command::new("git")
                .args(&["rm", "-f", &dep_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;

            Ok(())
        })
    }
}
