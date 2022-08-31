//! Update command
use crate::{cmd::Cmd, utils::CommandUtils};
use clap::{Parser, ValueHint};
use std::{path::PathBuf, process::Command};

#[derive(Debug, Clone, Parser)]
pub struct UpdateArgs {
    #[clap(
        help = "The path to the dependency you want to update.",
        value_hint = ValueHint::DirPath
    )]
    lib: Option<PathBuf>,
}

impl Cmd for UpdateArgs {
    type Output = ();

    fn run(self) -> eyre::Result<Self::Output> {
        let mut cmd = Command::new("git");
        cmd.args(&["submodule", "update", "--remote", "--init"]);
        // if a lib is specified, open it
        if let Some(lib) = self.lib {
            cmd.args(&["--", lib.display().to_string().as_str()]);
        }
        cmd.exec()?;

        Ok(())
    }
}
