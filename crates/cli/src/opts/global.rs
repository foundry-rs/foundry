use clap::{ArgAction, Parser};
use foundry_common::shell::{ColorChoice, OutputFormat, OutputMode, Shell, Verbosity};
use rayon::{current_num_threads, ThreadPoolBuilder};
use serde::{Deserialize, Serialize};

/// Global options.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Parser)]
pub struct GlobalOpts {
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
    /// - 5 (-vvvvv): Print execution and setup traces for all tests.
    #[clap(short, long, global = true, verbatim_doc_comment, conflicts_with = "quiet", action = ArgAction::Count, help_heading = "Display options")]
    pub verbosity: Verbosity,

    /// Do not print log messages.
    #[clap(
        short,
        long,
        global = true,
        alias = "silent",
        conflicts_with = "verbose",
        help_heading = "Display options"
    )]
    quiet: bool,

    /// Format log messages as JSON.
    #[clap(
        long,
        global = true,
        alias = "format-json",
        conflicts_with_all = &["quiet", "color"],
        help_heading = "Display options"
    )]
    json: bool,

    /// Log messages coloring.
    #[clap(long, global = true, value_enum, help_heading = "Display options")]
    color: Option<ColorChoice>,

    /// Number of threads to use.
    /// If 0, the number of threads will be equal to the number of logical CPUs.
    /// If set to a value greater than 0, it will use that number of threads capped at the number
    /// of logical CPUs.
    /// If not provided it will not spawn the global thread pool.
    #[clap(
        short,
        long,
        global = true,
        visible_alias = "threads",
        help_heading = "Concurrency options"
    )]
    jobs: Option<usize>,
}

impl GlobalOpts {
    /// Spawn a new global thread pool.
    pub fn try_spawn(self) -> Result<(), rayon::ThreadPoolBuildError> {
        if let Some(jobs) = self.try_jobs() {
            trace!(target: "forge::cli", "executing with {} max threads", jobs);
            ThreadPoolBuilder::new().num_threads(jobs).build_global()
        } else {
            // If `--jobs` is not provided, do not spawn the global thread pool.
            Ok(())
        }
    }

    /// Get the number of threads to use.
    ///
    /// Try to use the number of threads specified by `--jobs` if provided, otherwise use the number
    /// of logical CPUs. If running tests, use at least 2 threads.
    pub fn try_jobs(&self) -> Option<usize> {
        let num_threads = self.jobs.map(|jobs| {
            if jobs == 0 {
                current_num_threads()
            } else {
                jobs.min(current_num_threads())
            }
        });

        // If we are running tests, we want to use at least 2 threads.
        if cfg!(test) {
            if let Some(num_threads) = num_threads {
                return Some(num_threads.min(2));
            }
        }

        num_threads
    }

    /// Create a new shell instance.
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
