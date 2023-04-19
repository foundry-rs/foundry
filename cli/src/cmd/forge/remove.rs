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

    /// Override the up-to-date check.
    #[clap(short, long)]
    force: bool,
}
impl_figment_convert_basic!(RemoveArgs);

impl Cmd for RemoveArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let config = self.try_load_config_emit_warnings()?;
        let git_root =
            find_git_root_path(&config.__root.0).wrap_err("Unable to detect git root directory")?;
        let libs = config.install_lib_dir();

        let base_args: &[&str] = if self.force { &["rm", "--force"] } else { &["rm"] };
        for dep in &self.dependencies {
            let name = dep.name();
            let dep_path = libs.join(name);
            let path = dep_path.display().to_string();
            if !dep_path.exists() {
                eyre::bail!("Could not find dependency {name:?} in {path}");
            }

            println!("Removing {} in {path}, (url: {:?}, tag: {:?})", dep.name, dep.url, dep.tag);

            let mut args = base_args.to_vec();
            args.push(&path);
            Command::new("git").args(args).current_dir(&git_root).exec()?;
        }

        Ok(())
    }
}
