use clap::Parser;
use foundry_common::shell::{ColorChoice, OutputFormat, Shell, Verbosity};
use rayon::{current_num_threads, ThreadPoolBuilder};
use serde::{Deserialize, Serialize};

/// Global options.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Parser)]
pub struct GlobalOpts {
    /// Use verbose output.
    #[clap(long, global = true, conflicts_with = "quiet", help_heading = "Display options")]
    verbose: bool,

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
    #[clap(long, global = true, visible_alias = "threads", help_heading = "Concurrency options")]
    jobs: Option<usize>,
}

impl GlobalOpts {
    /// Spawn a new global thread pool.
    pub fn spawn(self) -> Result<(), rayon::ThreadPoolBuildError> {
        if let Some(jobs) = self.jobs {
            let threads = current_num_threads();
            let num_threads = if jobs == 0 { threads } else { jobs.min(threads) };
            return ThreadPoolBuilder::new().num_threads(num_threads).build_global();
        }

        Ok(())
    }

    /// Create a new shell instance.
    pub fn shell(self) -> Shell {
        let verbosity = match (self.verbose, self.quiet) {
            (true, false) => Verbosity::Verbose,
            (false, true) => Verbosity::Quiet,
            (false, false) => Verbosity::Normal,
            (true, true) => unreachable!(),
        };
        let color = self.json.then_some(ColorChoice::Never).or(self.color).unwrap_or_default();
        let format = match self.json {
            true => OutputFormat::Json,
            false => OutputFormat::Text,
        };

        Shell::new_with(format, color, verbosity)
    }
}
