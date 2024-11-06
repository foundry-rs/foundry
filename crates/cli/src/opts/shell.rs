use clap::{ArgAction, Parser};
use foundry_common::shell::{ColorChoice, OutputFormat, OutputMode, Shell};

// note: `verbose` and `quiet` cannot have `short` because of conflicts with multiple commands.

/// Global shell options.
#[derive(Clone, Copy, Debug, Parser)]
pub struct ShellOpts {
    /// Verbosity of the output.
    ///
    /// Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
    ///
    /// Depending on the context the verbosity levels may have different meanings.
    #[arg(long, short, verbatim_doc_comment, conflicts_with = "quiet", action = ArgAction::Count, help_heading = "Display options")]
    pub verbosity: u8,

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

    /// Format log messages as JSON.
    #[clap(
        long,
        global = true,
        alias = "format-json",
        conflicts_with_all = &["quiet", "color"],
        help_heading = "Display options"
    )]
    pub json: bool,

    /// Log messages coloring.
    #[clap(long, global = true, value_enum, help_heading = "Display options")]
    pub color: Option<ColorChoice>,
}

impl ShellOpts {
    pub fn shell(self) -> Shell {
        let mode = match self.quiet {
            true => OutputMode::Quiet,
            false => OutputMode::Normal,
        };
        let color = self.json.then_some(ColorChoice::Never).or(self.color).unwrap_or_default();
        let format = match self.json {
            true => OutputFormat::Json,
            false => OutputFormat::Text,
        };

        Shell::new_with(format, mode, color, self.verbosity)
    }
}
