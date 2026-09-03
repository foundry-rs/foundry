use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::{CommandUtils, Git, LoadConfig};
use foundry_config::impl_figment_convert_basic;
use std::{path::PathBuf, process::Command};

/// CLI arguments for `forge reinit`.
#[derive(Clone, Debug, Parser)]
pub struct ReinitArgs {
    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
}
impl_figment_convert_basic!(ReinitArgs);

impl ReinitArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let root = Git::root_of(&config.root)?;
        let git = Git::new(&root);

        Command::new("git")
            .current_dir(&root)
            .args(["submodule", "deinit", "--force", "."])
            .exec()?;
        git.submodule_update(false, false, false, true, std::iter::empty::<PathBuf>())
    }
}
