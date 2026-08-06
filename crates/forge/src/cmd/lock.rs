use crate::Lockfile;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::utils::{Git, LoadConfig};
use foundry_config::impl_figment_convert_basic;
use std::{fmt::Write, path::PathBuf};

/// CLI arguments for `forge lock`.
#[derive(Clone, Debug, Parser)]
pub struct LockArgs {
    /// Check that foundry.lock matches the installed dependency revisions.
    #[arg(long, required = true)]
    check: bool,

    /// The project's root path.
    ///
    /// By default root of the Git repository, if in one,
    /// or the current working directory.
    #[arg(long, value_hint = ValueHint::DirPath, value_name = "PATH")]
    root: Option<PathBuf>,
}
impl_figment_convert_basic!(LockArgs);

impl LockArgs {
    pub fn run(self) -> Result<()> {
        let config = self.load_config()?;
        let git = Git::new(&config.root);
        let mut lockfile = Lockfile::new(&config.root).with_git(&git);
        let mismatches = lockfile.check()?;
        if mismatches.is_empty() {
            return Ok(());
        }

        let mut message = String::from("foundry.lock does not match installed dependencies:");
        for mismatch in mismatches {
            write!(message, "\n  {mismatch}")?;
        }
        Err(eyre::eyre!(message))
    }
}
