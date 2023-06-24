use crate::{
    cmd::{Cmd, LoadConfig},
    opts::Dependency,
    utils::CommandUtils,
};
use clap::{Parser, ValueHint};
use eyre::WrapErr;
use foundry_config::{find_git_root_path, impl_figment_convert_basic};
use std::{fs, path::PathBuf, process::Command};

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
        let git_root =
            find_git_root_path(&config.__root.0).wrap_err("Unable to detect git root directory")?;
        let libs = config.install_lib_dir();
        let git_modules = git_root.join(".git/modules");

        let base_args: &[&str] = if self.force { &["rm", "--force"] } else { &["rm"] };
        for dep in &self.dependencies {
            eprintln!("rm {dep:#?}");
            let name = dep.name();
            let dep_path = libs.join(name);
            let path = dep_path.display().to_string();
            let rel_path = dep_path
                .strip_prefix(&git_root)
                .wrap_err("Library directory is not relative to the repository root")?;
            if !dep_path.exists() {
                eyre::bail!("Could not find dependency {name:?} in {path}");
            }

            println!("Removing '{}' in {path}, (url: {:?}, tag: {:?})", dep.name, dep.url, dep.tag);

            // completely remove the submodule:
            // git rm <path> && rm -rf .git/modules/<path>
            let mut args = base_args.to_vec();
            args.push(&path);
            Command::new("git").args(args).current_dir(&git_root).exec()?;

            fs::remove_dir_all(git_modules.join(rel_path))
                .wrap_err("Failed removing .git submodule directory")?;
        }

        Ok(())
    }
}
