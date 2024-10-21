use clap::Parser;
use foundry_common::shell::{ColorChoice, Shell, Verbosity};

// note: `verbose` and `quiet` cannot have `short` because of conflicts with multiple commands.

/// Global shell options.
#[derive(Clone, Copy, Debug, Parser)]
pub struct ShellOpts {
    /// Use verbose output.
    #[clap(long, global = true, conflicts_with = "quiet", help_heading = "Display options")]
    pub verbose: bool,

    /// Do not print log messages.
    #[clap(
        short,
        long,
        global = true,
        alias = "silent",
        conflicts_with = "verbose",
        help_heading = "Display options"
    )]
    pub quiet: bool,

    /// Log messages coloring.
    #[clap(long, global = true, value_enum, help_heading = "Display options")]
    pub color: Option<ColorChoice>,
}

impl ShellOpts {
    pub fn shell(self) -> Shell {
        let verbosity = match (self.verbose, self.quiet) {
            (true, false) => Verbosity::Verbose,
            (false, true) => Verbosity::Quiet,
            (false, false) => Verbosity::Normal,
            (true, true) => unreachable!(),
        };
        Shell::new_with(self.color.unwrap_or_default(), verbosity)
    }
}
