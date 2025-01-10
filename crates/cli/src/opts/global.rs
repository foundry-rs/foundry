use clap::{ArgAction, Parser};
use foundry_common::shell::{ColorChoice, OutputFormat, OutputMode, Shell, Verbosity};
use serde::{Deserialize, Serialize};

/// Global arguments for the CLI.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Parser)]
pub struct GlobalArgs {
    /// Verbosity level of the log messages.
    ///
    /// Pass multiple times to increase the verbosity (e.g. -v, -vv, -vvv).
    ///
    /// Depending on the context the verbosity levels have different meanings.
    ///
    /// For example, the verbosity levels of the EVM are:
    /// - 2 (-vv): Print logs for all tests.
    /// - 3 (-vvv): Print execution traces for failing tests.
    /// - 4 (-vvvv): Print execution traces for all tests, and setup traces for failing tests.
    /// - 5 (-vvvvv): Print execution and setup traces for all tests, including storage changes.
    #[arg(help_heading = "Display options", global = true, short, long, verbatim_doc_comment, conflicts_with = "quiet", action = ArgAction::Count)]
    verbosity: Verbosity,

    /// Do not print log messages.
    #[arg(help_heading = "Display options", global = true, short, long, alias = "silent")]
    quiet: bool,

    /// Format log messages as JSON.
    #[arg(help_heading = "Display options", global = true, long, alias = "format-json", conflicts_with_all = &["quiet", "color"])]
    json: bool,

    /// The color of the log messages.
    #[arg(help_heading = "Display options", global = true, long, value_enum)]
    color: Option<ColorChoice>,

    /// Number of threads to use. Specifying 0 defaults to the number of logical cores.
    #[arg(global = true, long, short = 'j', visible_alias = "jobs")]
    threads: Option<usize>,
}

impl GlobalArgs {
    /// Initialize the global options.
    pub fn init(&self) -> eyre::Result<()> {
        // Set the global shell.
        self.shell().set();

        // Initialize the thread pool only if `threads` was requested to avoid unnecessary overhead.
        if self.threads.is_some() {
            self.force_init_thread_pool()?;
        }

        Ok(())
    }

    /// Create a new shell instance.
    pub fn shell(&self) -> Shell {
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

    /// Initialize the global thread pool.
    pub fn force_init_thread_pool(&self) -> eyre::Result<()> {
        init_thread_pool(self.threads.unwrap_or(0))
    }
}

/// Initialize the global thread pool.
pub fn init_thread_pool(threads: usize) -> eyre::Result<()> {
    rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("foundry-{i}"))
        .num_threads(threads)
        .build_global()?;
    Ok(())
}
