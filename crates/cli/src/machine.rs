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
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Like [`bail_machine_usage`] but attaches structured `details` so agents
/// can react without parsing the prose `message`.
pub fn bail_machine_usage_with_details(
    message: impl Into<String>,
    details: serde_json::Value,
) -> ! {
    let envelope = JsonEnvelope::error(
        JsonMessage::error(diagnostic::cli::USAGE_INVALID, message).with_details(details),
    );
    let _ = print_json(&envelope);
    std::process::exit(ExitCode::Usage.to_i32());
}

/// Fallback envelope emitter for an untyped `eyre::Report`. Always tags
/// `cli.unknown` and preserves the eyre cause chain in `details.cause_chain`.
/// The process exit code is the caller's responsibility.
pub fn report_machine_error(report: &eyre::Report) {
    let cause_chain: Vec<String> = report.chain().map(ToString::to_string).collect();
    let message = cause_chain.first().cloned().unwrap_or_else(|| report.to_string());
    let envelope = JsonEnvelope::error(
        JsonMessage::error(diagnostic::cli::UNKNOWN, message)
            .with_details(json!({ "cause_chain": cause_chain })),
    );
    let _ = print_json(&envelope);
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

    /// `report_machine_error` always tags `cli.unknown` and preserves the
    /// full eyre cause chain in `errors[0].details.cause_chain`.
    #[test]
    fn report_machine_error_uses_cli_unknown_and_preserves_cause_chain() {
        use eyre::WrapErr as _;
        let leaf: eyre::Report = eyre::eyre!("solc: missing semicolon");
        let report: eyre::Report = Result::<(), _>::Err(leaf)
            .wrap_err("compile failed")
            .wrap_err("build failed")
            .unwrap_err();

        let cause_chain: Vec<String> = report.chain().map(ToString::to_string).collect();
        let message = cause_chain.first().cloned().unwrap();
        let envelope = JsonEnvelope::error(
            JsonMessage::error(diagnostic::cli::UNKNOWN, message)
                .with_details(json!({ "cause_chain": cause_chain })),
        );

        assert!(!envelope.success);
        assert_eq!(envelope.errors.len(), 1);
        assert_eq!(envelope.errors[0].code, diagnostic::cli::UNKNOWN);
        assert_eq!(envelope.errors[0].message, "build failed");
        let details = envelope.errors[0].details.as_ref().expect("details");
        let chain = details.get("cause_chain").and_then(|v| v.as_array()).expect("chain");
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], "build failed");
        assert_eq!(chain[2], "solc: missing semicolon");
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
