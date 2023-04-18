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
    #[clap(help = "The path to the dependency you want to remove.")]
    dependencies: Vec<Dependency>,
    #[clap(
        help = "The project's root path. By default, this is the root directory of the current Git repository or the current working directory if it is not part of a Git repository",
        long,
        value_hint = ValueHint::DirPath,
        value_name = "PATH"
    )]
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

        for dep in &self.dependencies {
            let name = dep.name();
            let dep_path = libs.join(name);
            if !dep_path.exists() {
                eyre::bail!("Could not find dependency {name:?} in {}", dep_path.display());
            }

            println!(
                "Removing {} in {dep_path:?}, (url: {:?}, tag: {:?})",
                dep.name, dep.url, dep.tag
            );

            Command::new("git")
                .args(["rm", &dep_path.display().to_string()])
                .current_dir(&git_root)
                .exec()?;
        }

        Ok(())
    }
}
