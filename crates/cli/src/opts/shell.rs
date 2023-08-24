use crate::shell::{ColorChoice, Shell, Verbosity};
use clap::Parser;

/// Shell options.
#[derive(Clone, Copy, Debug, Parser)]
pub struct ShellOptions {
    /// Use verbose output.
    #[clap(long, short, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Do not print log messages.
    #[clap(long, short, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Log messages coloring.
    #[clap(long, global = true)]
    pub color: ColorChoice,
}

impl ShellOptions {
    pub fn shell(self) -> Shell {
        let verbosity = match (self.verbose, self.quiet) {
            (true, false) => Verbosity::Verbose,
            (false, true) => Verbosity::Quiet,
            (false, false) => Verbosity::Normal,
            (true, true) => unreachable!(),
        };
        Shell::new_with(self.color, verbosity)
    }

    pub fn set_global_shell(self) {
        self.shell().set();
    }
}
