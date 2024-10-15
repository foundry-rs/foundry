use clap::Parser;
use foundry_common::shell::{ColorChoice, Shell, Verbosity};

// note: `verbose` and `quiet` cannot have `short` because of conflicts with multiple commands.

/// Global shell options.
#[derive(Clone, Copy, Debug, Parser)]
pub struct ShellOpts {
    /// Use verbose output.
    #[clap(long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Do not print log messages.
    #[clap(short, long, global = true, alias = "silent", conflicts_with = "verbose")]
    pub quiet: bool,

    /// Format output as JSON.
    #[clap(long, global = true)]
    pub json: bool,

    /// Log messages coloring.
    #[clap(long, global = true, value_enum)]
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
        let format = match self.json {
            true => foundry_common::shell::Format::Json,
            false => foundry_common::shell::Format::Text,
        };
        Shell::new_with(self.color.unwrap_or_default(), verbosity, format)
    }
}
