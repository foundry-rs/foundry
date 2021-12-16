//! Various forge shell commands

pub mod help;
pub mod list;
use crate::Shell;

/// A trait for forge shell commands
pub trait Cmd: structopt::StructOpt + Sized {
    fn run(self, shell: &mut Shell) -> eyre::Result<()>;

    fn run_str(shell: &mut Shell, args: &[String]) -> eyre::Result<()> {
        let args = Self::from_iter_safe(args)?;
        args.run(shell)
    }
}
