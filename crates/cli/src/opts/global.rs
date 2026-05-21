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
        if std::env::args().take_while(|a| a != "--").any(|a| a == "--markdown-help") {
            // Pre-parse: `Shell` is not initialized yet, so `sh_*` is unavailable.
            #[allow(clippy::disallowed_macros)]
            {
                eprintln!(
                    "note: `--markdown-help` is intended for human documentation; \
                     agents should use `--introspect` for machine-readable command discovery",
                );
            }
            foundry_cli_markdown::print_help_markdown::<C>();
            std::process::exit(0);
        }
    }

    /// Check if `--introspect` was passed and print the introspection document as JSON, then exit.
    ///
    /// Must run **before** clap parsing so required subcommands or args don't block discovery.
    /// Uses `CommandRegistry::EMPTY`; for per-command metadata, call
    /// [`check_introspect_with`](Self::check_introspect_with) with a populated registry.
    pub fn check_introspect<C: clap::CommandFactory>() {
        if !pre_parse_flag_present("--introspect") {
            return;
        }
        emit_introspect_and_exit(C::command(), &crate::introspect::CommandRegistry::EMPTY);
    }

    /// Like [`check_introspect`](Self::check_introspect) but uses an explicit
    /// [`Command`](clap::Command) and [`CommandRegistry`](crate::introspect::CommandRegistry).
    pub fn check_introspect_with(
        command: clap::Command,
        registry: &crate::introspect::CommandRegistry,
    ) {
        if pre_parse_flag_present("--introspect") {
            emit_introspect_and_exit(command, registry);
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

fn emit_introspect_and_exit(
    command: clap::Command,
    registry: &crate::introspect::CommandRegistry,
) -> ! {
    let json = crate::introspect::render_introspect_document(&command, registry);
    // Pre-parse: `Shell` is not initialized yet, so `sh_*` is unavailable.
    #[allow(clippy::disallowed_macros)]
    {
        println!("{json}");
    }
    std::process::exit(0);
}

/// Returns whether `flag` is present among the binary's leading top-level options.
///
/// The scan ends at the first `--` separator, the first subcommand or
/// positional token, or the first non-UTF-8 token (none of which can match
/// the ASCII flags this helper supports). Values of known value-taking
/// global options are skipped, so e.g. `forge --color always --introspect`
/// and `forge -j 4 --introspect` still match.
fn pre_parse_flag_present(flag: &str) -> bool {
    pre_parse_flag_present_in(std::env::args_os().skip(1), flag)
}

/// Value-taking global options declared on [`GlobalArgs`]; their following
/// argv token is the value, not a flag, and must not be considered by the
/// pre-parse scan. Must stay in sync with the `#[arg(... long, short)]`
/// declarations above.
const VALUE_TAKING_GLOBAL_OPTIONS: &[&str] = &["--color", "-j", "--threads", "--jobs"];

fn pre_parse_flag_present_in<I>(args: I, flag: &str) -> bool
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        if a == "--" {
            return false;
        }
        let Some(s) = a.to_str() else {
            // Non-UTF-8 token: cannot be `--introspect` / `--markdown-help`
            // and most likely a positional value — stop scanning here.
            return false;
        };
        if !s.starts_with('-') {
            return false;
        }
        if s == flag {
            return true;
        }
        // Skip the value of `--opt VALUE` form; `--opt=VALUE` is one token.
        if !s.contains('=') && VALUE_TAKING_GLOBAL_OPTIONS.contains(&s) {
            iter.next();
        }
    }
    false
}

/// Initialize the global thread pool.
pub fn init_thread_pool(threads: usize) -> eyre::Result<()> {
    rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("foundry-{i}"))
        .num_threads(threads)
        .build_global()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn argv(slice: &[&str]) -> Vec<OsString> {
        slice.iter().map(|s| OsString::from(*s)).collect()
    }

    #[test]
    fn pre_parse_flag_present_matches_top_level_flags() {
        for case in [
            &["--introspect"][..],
            &["--color", "always", "--introspect"],
            &["-j", "4", "--introspect"],
            &["--color=always", "--introspect"],
            &["--quiet", "--introspect"],
        ] {
            assert!(pre_parse_flag_present_in(argv(case), "--introspect"), "case: {case:?}");
        }
    }

    #[test]
    fn pre_parse_flag_present_ignores_values_and_subcommands() {
        for case in [
            &[][..],
            &["--", "--introspect"],
            &["test", "--introspect"],
            // `cast call ADDR "method(string)" --data --introspect`
            &["call", "ADDR", "method(string)", "--data", "--introspect"],
        ] {
            assert!(!pre_parse_flag_present_in(argv(case), "--introspect"), "case: {case:?}");
        }
    }
}
