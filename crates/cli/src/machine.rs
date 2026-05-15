//! Machine mode (`--machine`) — agent-contract output selector.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md) §10. This
//! module ships the **runtime layer** of `--machine`: pre-parse detection,
//! clap-error interception, and the canonical [`ExitCode`] mapping for
//! pre-command exits.
//!
//! Runtime guarantees, regardless of which command is invoked:
//!
//! - color is disabled (wired through [`crate::opts::GlobalArgs::shell`]),
//! - parse / usage failures are wrapped in an error envelope (`cli.usage.invalid`, exit `2`),
//! - `--help` / `--version` are wrapped in a success envelope (exit `0`).
//!
//! Per-command behavior — emitting only the declared
//! [`output_mode`](crate::introspect::OutputMode), suppressing progress
//! bars and interactive prompts, returning the canonical [`ExitCode`] for
//! the failure category — is opt-in and adopted incrementally.
//!
//! The flag is detected before clap parsing — see [`check_machine`] — so
//! the mode is known by the time clap errors need to be intercepted.

use crate::{
    diagnostic,
    exit_code::ExitCode,
    json::{JsonEnvelope, JsonMessage, print_json},
};
use clap::{CommandFactory, Parser, error::ErrorKind};
use serde_json::json;
use std::{
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};

static MACHINE_MODE: AtomicBool = AtomicBool::new(false);

/// Returns whether `--machine` was set on the current invocation.
///
/// Only meaningful after [`check_machine`] has run.
pub fn is_machine() -> bool {
    MACHINE_MODE.load(Ordering::Relaxed)
}

/// Force machine mode on or off. Intentionally crate-private: production
/// activation goes through [`check_machine`] (pre-parse) or
/// [`crate::opts::GlobalArgs::init`] (post-parse re-sync).
pub(crate) fn set_machine(on: bool) {
    MACHINE_MODE.store(on, Ordering::Relaxed);
}

/// Pre-parse scan for `--machine`.
///
/// Runs before clap parsing so the flag is visible while intercepting parse
/// errors. Honors `--machine`'s clap-global declaration, so `cast call
/// --machine --help` also flips the mode.
pub fn check_machine() {
    if crate::opts::pre_parse_global_flag_present("--machine") {
        set_machine(true);
    }
}

/// Parse arguments, intercepting clap errors when machine mode is on.
///
/// Replaces `T::parse()` at binary entry points. Under `--machine`, parse
/// errors and `--help` / `--version` are converted into a structured
/// [`JsonEnvelope`] on stdout and the process exits with the appropriate
/// [`ExitCode`]. Without `--machine`, behaves exactly like
/// [`Parser::parse`].
pub fn parse_or_exit<T: Parser + CommandFactory>() -> T {
    if is_machine() {
        // `GlobalArgs::init()` (which calls `yansi::disable()`) hasn't run
        // yet; force `ColorChoice::Never` on the command so clap's rendered
        // help / error text never embeds ANSI escapes in the envelope.
        let mut cmd = T::command().color(clap::ColorChoice::Never);
        let mut matches = match cmd.try_get_matches_from_mut(std::env::args_os()) {
            Ok(m) => m,
            Err(err) => handle_machine_clap_error(err),
        };
        match T::from_arg_matches_mut(&mut matches) {
            Ok(t) => t,
            Err(err) => handle_machine_clap_error(err),
        }
    } else {
        match T::try_parse() {
            Ok(t) => t,
            Err(err) => err.exit(),
        }
    }
}

/// Convert a clap error into a structured machine-mode envelope and exit.
///
/// - `DisplayHelp` / `DisplayVersion` (explicit `--help` / `--version`) → success envelope wrapping
///   clap's already-rendered, context-aware text (so e.g. `cast call --help` yields `cast call`
///   help, not root help), exit `0`.
/// - Everything else (parse errors, missing subcommand, missing required arg, conflict, including
///   `DisplayHelpOnMissingArgumentOrSubcommand`, which is clap's "render help because args were
///   missing" — i.e. a usage failure, not a help request) → error envelope with
///   `cli.usage.invalid`, exit `2`.
fn handle_machine_clap_error(err: clap::Error) -> ! {
    let exit = exit_code_for_clap_error(&err);
    match err.kind() {
        ErrorKind::DisplayHelp => {
            let rendered = err.render().to_string();
            let envelope = JsonEnvelope::success(json!({ "help": rendered }));
            let _ = print_json(&envelope);
        }
        ErrorKind::DisplayVersion => {
            let rendered = err.render().to_string();
            let envelope = JsonEnvelope::success(json!({ "version": rendered }));
            let _ = print_json(&envelope);
        }
        _ => {
            // Includes `DisplayHelpOnMissingArgumentOrSubcommand`: clap
            // rendered help text, but the underlying cause is a missing
            // required arg or subcommand — a usage failure by contract.
            let message = err.to_string();
            let envelope =
                JsonEnvelope::error(JsonMessage::error(diagnostic::cli::USAGE_INVALID, message));
            let _ = print_json(&envelope);
        }
    }
    std::process::exit(exit.to_i32());
}

/// Maps a clap error kind to the canonical [`ExitCode`].
///
/// Only `DisplayHelp` and `DisplayVersion` (explicit `--help` / `--version`)
/// are successes; everything else (parse errors, missing subcommand,
/// missing required arg, conflict, `DisplayHelpOnMissingArgumentOrSubcommand`)
/// is `Usage`.
fn exit_code_for_clap_error(err: &clap::Error) -> ExitCode {
    match err.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::Success,
        _ => ExitCode::Usage,
    }
}

/// Emit a `cli.usage.invalid` envelope on stdout and exit with
/// [`ExitCode::Usage`] (`2`). Use at call sites that intentionally reject
/// a flag combination under `--machine`.
pub fn bail_machine_usage(message: impl Into<String>) -> ! {
    let envelope = JsonEnvelope::error(JsonMessage::error(diagnostic::cli::USAGE_INVALID, message));
    let _ = print_json(&envelope);
    std::process::exit(ExitCode::Usage.to_i32());
}

/// Emit a structured error envelope on stdout for an `eyre::Report`.
///
/// Used by binary entry points to wrap an uncaught command failure as a
/// terminal envelope. The diagnostic code is best-effort, classified from
/// the report's cause chain via [`diagnostic_code_for_report`]; the process
/// exit code is the caller's responsibility (typically [`ExitCode::GenericError`]).
pub fn report_machine_error(report: &eyre::Report) {
    let message = format!("{report}");
    let envelope =
        JsonEnvelope::error(JsonMessage::error(diagnostic_code_for_report(report), message));
    let _ = print_json(&envelope);
}

/// Conservative classification of an `eyre::Report` into a stable diagnostic
/// code.
///
/// Stable codes are part of the agent contract; over-specific
/// misclassification is worse than the catch-all. Only a small set of
/// high-confidence keyword matches escape the [`diagnostic::cli::UNKNOWN`]
/// fallback. Typed call sites should emit specific codes directly rather
/// than relying on this helper.
pub fn diagnostic_code_for_report(report: &eyre::Report) -> &'static str {
    let mut buf = String::new();
    for cause in report.chain() {
        let _ = writeln!(buf, "{cause}");
    }
    let lower = buf.to_lowercase();

    if lower.contains("interrupted") || lower.contains("sigint") || lower.contains("sigterm") {
        return diagnostic::cli::INTERRUPTED;
    }
    if lower.contains("foundry.toml") {
        return diagnostic::config::INVALID;
    }
    if lower.contains("solc") {
        return diagnostic::compiler::SOLC_ERROR;
    }
    if lower.contains("vyper") {
        return diagnostic::compiler::VYPER_ERROR;
    }
    diagnostic::cli::UNKNOWN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_flag_default_off() {
        set_machine(false);
        assert!(!is_machine());
    }

    #[test]
    fn machine_flag_can_be_toggled() {
        set_machine(true);
        assert!(is_machine());
        set_machine(false);
        assert!(!is_machine());
    }

    #[derive(Debug, Parser)]
    #[command(name = "demo", version = "0.1.0")]
    struct Demo {
        #[arg(long)]
        name: Option<String>,
        #[command(subcommand)]
        cmd: Option<DemoSub>,
    }

    #[derive(Debug, clap::Subcommand)]
    enum DemoSub {
        /// Build the project.
        Build {
            #[arg(long)]
            path: Option<String>,
        },
    }

    #[derive(Debug, Parser)]
    #[command(
        name = "strict",
        version = "0.1.0",
        subcommand_required = true,
        arg_required_else_help = true
    )]
    struct Strict {
        #[command(subcommand)]
        cmd: StrictSub,
    }

    #[derive(Debug, clap::Subcommand)]
    enum StrictSub {
        Run,
    }

    #[test]
    fn diagnostic_classifier_picks_compiler_for_solc_errors() {
        let r: eyre::Report = eyre::eyre!("solc compilation failed");
        assert_eq!(diagnostic_code_for_report(&r), diagnostic::compiler::SOLC_ERROR);
    }

    #[test]
    fn diagnostic_classifier_falls_back_to_cli_unknown_for_rpc_failures() {
        // Generic RPC errors stay `cli.unknown` — typed call sites should
        // emit `network.rpc.*` codes themselves when they have the context
        // to classify accurately.
        let r: eyre::Report = eyre::eyre!("RPC connection timeout");
        assert_eq!(diagnostic_code_for_report(&r), diagnostic::cli::UNKNOWN);
    }

    #[test]
    fn diagnostic_classifier_falls_back_to_cli_unknown() {
        let r: eyre::Report = eyre::eyre!("something unexpected went wrong");
        assert_eq!(diagnostic_code_for_report(&r), diagnostic::cli::UNKNOWN);
    }

    #[test]
    fn clap_error_kinds_map_to_exit_codes() {
        let bad = Demo::try_parse_from(["demo", "--unknown"]).unwrap_err();
        assert_eq!(exit_code_for_clap_error(&bad), ExitCode::Usage);

        let help = Demo::try_parse_from(["demo", "--help"]).unwrap_err();
        assert_eq!(exit_code_for_clap_error(&help), ExitCode::Success);

        let version = Demo::try_parse_from(["demo", "--version"]).unwrap_err();
        assert_eq!(exit_code_for_clap_error(&version), ExitCode::Success);
    }

    /// `DisplayHelpOnMissingArgumentOrSubcommand` is clap's "render help
    /// because args were missing" — a usage failure, not a help request.
    /// The agent contract maps it to [`ExitCode::Usage`], not `Success`.
    #[test]
    fn missing_required_subcommand_classifies_as_usage() {
        let err = Strict::try_parse_from(["strict"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
        assert_eq!(exit_code_for_clap_error(&err), ExitCode::Usage);
    }

    /// Subcommand `--help` must surface the **subcommand's** help, not the
    /// root help. This is the contract test for the I3 fix: rendering
    /// flows through `err.render()` instead of `T::command().render_help()`.
    #[test]
    fn subcommand_help_preserves_command_context() {
        let err = Demo::try_parse_from(["demo", "build", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
        let rendered = err.render().to_string();
        assert!(
            rendered.contains("Build the project"),
            "subcommand help should mention the subcommand description, got: {rendered}"
        );
        // Root-only subcommand list MUST NOT appear in subcommand help.
        assert!(
            !rendered.contains("Usage: demo [OPTIONS]"),
            "subcommand help leaked root usage: {rendered}"
        );
    }
}
