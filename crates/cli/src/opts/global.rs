use clap::{ArgAction, Parser};
use foundry_common::{
    shell::{ColorChoice, OutputFormat, OutputMode, Shell, Verbosity},
    version::{IS_NIGHTLY_VERSION, NIGHTLY_VERSION_WARNING_MESSAGE},
};
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
    /// - 5 (-vvvvv): Print execution and setup traces for all tests, including storage changes and
    ///   backtraces with line numbers.
    #[arg(help_heading = "Display options", global = true, short, long, verbatim_doc_comment, conflicts_with = "quiet", action = ArgAction::Count)]
    verbosity: Verbosity,

    /// Do not print log messages.
    #[arg(help_heading = "Display options", global = true, short, long, alias = "silent")]
    quiet: bool,

    /// Format log messages as JSON.
    #[arg(help_heading = "Display options", global = true, long, alias = "format-json", conflicts_with_all = &["quiet", "color"])]
    json: bool,

    /// Format log messages as Markdown.
    #[arg(
        help_heading = "Display options",
        global = true,
        long,
        alias = "markdown",
        conflicts_with = "json"
    )]
    md: bool,

    /// The color of the log messages.
    #[arg(help_heading = "Display options", global = true, long, value_enum)]
    color: Option<ColorChoice>,

    /// Number of threads to use. Specifying 0 defaults to the number of logical cores.
    #[arg(global = true, long, short = 'j', visible_alias = "jobs")]
    threads: Option<usize>,
}

impl GlobalArgs {
    /// Check if `--markdown-help` was passed and print CLI reference as Markdown, then exit.
    ///
    /// This must be called **before** parsing arguments, since commands with required
    /// subcommands would fail parsing before the flag is checked.
    pub fn check_markdown_help<C: clap::CommandFactory>() {
        if std::env::args().any(|arg| arg == "--markdown-help") {
            foundry_cli_markdown::print_help_markdown::<C>();
            std::process::exit(0);
        }
    }

    /// Initialize the global options.
    pub fn init(&self) -> eyre::Result<()> {
        // Set the global shell.
        let shell = self.shell();
        // Argument takes precedence over the env var global color choice.
        match shell.color_choice() {
            ColorChoice::Auto => {}
            ColorChoice::Always => yansi::enable(),
            ColorChoice::Never => yansi::disable(),
        }
        shell.set();

        // Initialize the thread pool only if `threads` was requested to avoid unnecessary overhead.
        if self.threads.is_some() {
            self.force_init_thread_pool()?;
        }

        // Display a warning message if the current version is not stable.
        if IS_NIGHTLY_VERSION
            && !self.json
            && std::env::var_os("FOUNDRY_DISABLE_NIGHTLY_WARNING").is_none()
        {
            let _ = sh_warn!("{}", NIGHTLY_VERSION_WARNING_MESSAGE);
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
        let format = if self.json {
            OutputFormat::Json
        } else if self.md {
            OutputFormat::Markdown
        } else {
            OutputFormat::Text
        };

        Shell::new_with(format, mode, color, self.verbosity)
    }

    /// Initialize the global thread pool.
    pub fn force_init_thread_pool(&self) -> eyre::Result<()> {
        init_thread_pool(self.threads.unwrap_or(0))
    }

    /// Creates a new tokio runtime.
    #[track_caller]
    pub fn tokio_runtime(&self) -> tokio::runtime::Runtime {
        let mut builder = tokio::runtime::Builder::new_multi_thread();
        if let Some(threads) = self.threads
            && threads > 0
        {
            builder.worker_threads(threads);
        }
        builder.enable_all().build().expect("failed to create tokio runtime")
    }

    /// Creates a new tokio runtime and blocks on the future.
    #[track_caller]
    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.tokio_runtime().block_on(future)
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
