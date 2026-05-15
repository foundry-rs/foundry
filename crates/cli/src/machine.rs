//! Machine mode (`--machine`) — agent-contract output selector.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md) §10. When a
//! command is invoked with `--machine`:
//!
//! - it emits its declared [`output_mode`](crate::introspect::OutputMode) only,
//! - it never writes color, progress bars, or interactive prompts to stdout,
//! - parse, usage, version, and help failures are structured as envelopes,
//! - process-exit failures map to the canonical [`ExitCode`] enum.
//!
//! The flag is detected before clap parsing — see [`check_machine`] — so the
//! mode is known by the time clap errors need to be intercepted.
//!
//! Adoption of machine-mode output by individual commands lands in follow-up
//! PRs. PR 2 wires the runtime infrastructure (flag detection, error
//! interception, exit-code mapping); per-command envelope emission is
//! deferred.

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

/// Force machine mode on. Intended for tests and for [`check_machine`].
#[doc(hidden)]
pub fn set_machine(on: bool) {
    MACHINE_MODE.store(on, Ordering::Relaxed);
}

/// Pre-parse scan for `--machine`.
///
/// Mirrors `check_introspect` / `check_markdown_help`: runs before clap
/// parsing so the flag is visible while intercepting parse errors. Does NOT
/// exit — machine mode persists for the rest of the run.
pub fn check_machine() {
    if std::env::args().any(|a| a == "--machine") {
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
    match T::try_parse() {
        Ok(t) => t,
        Err(err) => {
            if is_machine() {
                handle_machine_clap_error::<T>(err)
            } else {
                err.exit()
            }
        }
    }
}

fn handle_machine_clap_error<T: CommandFactory>(err: clap::Error) -> ! {
    let kind = err.kind();
    match kind {
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let mut cmd = T::command();
            let help = cmd.render_help().to_string();
            let envelope = JsonEnvelope::success(json!({ "help": help }));
            let _ = print_json(&envelope);
            std::process::exit(ExitCode::Success.to_i32());
        }
        ErrorKind::DisplayVersion => {
            let cmd = T::command();
            let envelope = JsonEnvelope::success(json!({
                "version": cmd.get_version().unwrap_or(""),
                "long_version": cmd.get_long_version().unwrap_or(""),
            }));
            let _ = print_json(&envelope);
            std::process::exit(ExitCode::Success.to_i32());
        }
        _ => {
            let message = err.to_string();
            let envelope = JsonEnvelope::error(
                JsonMessage::error(diagnostic::cli::USAGE_INVALID, message)
                    .with_details(json!({ "clap_error_kind": format!("{kind:?}") })),
            );
            let _ = print_json(&envelope);
            std::process::exit(ExitCode::Usage.to_i32());
        }
    }
}

/// Exit code that maps a clap error kind to the canonical table.
///
/// Useful for callers that have a `clap::Error` they want to classify
/// without going through [`parse_or_exit`].
pub fn exit_code_for_clap_error(err: &clap::Error) -> ExitCode {
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        | ErrorKind::DisplayVersion => ExitCode::Success,
        _ => ExitCode::Usage,
    }
}

/// Emit a structured error envelope on stdout for an `eyre::Report`.
///
/// Adoption PRs use this at command-execution failure sites under
/// `--machine`. PR 2 ships the helper but does not yet route command-error
/// failures through it; that is part of per-command envelope adoption.
pub fn report_machine_error(report: &eyre::Report) {
    let message = format!("{report}");
    let envelope = JsonEnvelope::error(JsonMessage::error(diagnostic::cli::UNKNOWN, message));
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
}
