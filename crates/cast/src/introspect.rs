//! Stable, agent-facing metadata for the `cast` command tree.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md). The registry
//! pins `command_id`s and machine-mode capabilities for adopted commands.

use foundry_cli::{
    ExitCode,
    introspect::{
        Capabilities, CommandMeta, CommandRegistry, ExitCodeInfo, OutputMode, RegistryEntry,
        SideEffects,
    },
};
use std::borrow::Cow;

/// Stable schema id for the `cast call` envelope payload.
pub const CALL_RESULT_SCHEMA: &str = "foundry:cast.call@v1";

/// Exit codes `cast call` may emit under `--machine`. Declared explicitly so
/// agents don't have to assume "inherits global defaults"; codes not in this
/// list are not produced by this command.
static CALL_EXIT_CODES: &[ExitCodeInfo] = &[
    ExitCodeInfo {
        code: ExitCode::Success.to_i32(),
        name: Cow::Borrowed(ExitCode::Success.name()),
        description: Cow::Borrowed("Call succeeded; envelope `success: true` with raw hex."),
    },
    ExitCodeInfo {
        code: ExitCode::GenericError.to_i32(),
        name: Cow::Borrowed(ExitCode::GenericError.name()),
        description: Cow::Borrowed(
            "Unclassified failure outside the typed paths (envelope `cli.unknown`).",
        ),
    },
    ExitCodeInfo {
        code: ExitCode::Usage.to_i32(),
        name: Cow::Borrowed(ExitCode::Usage.name()),
        description: Cow::Borrowed(
            "Flag combination rejected under `--machine` (envelope `cli.usage.invalid`).",
        ),
    },
    ExitCodeInfo {
        code: ExitCode::Network.to_i32(),
        name: Cow::Borrowed(ExitCode::Network.name()),
        description: Cow::Borrowed(
            "Provider construction or `eth_call` failed (envelope `network.rpc.error`).",
        ),
    },
    ExitCodeInfo {
        code: ExitCode::User.to_i32(),
        name: Cow::Borrowed(ExitCode::User.name()),
        description: Cow::Borrowed("Wallet / keystore resolution failed in non-interactive setup."),
    },
];

static ENTRIES: &[RegistryEntry] = &[RegistryEntry {
    path: &["call"],
    meta: CommandMeta {
        command_id: Some("cast.call"),
        capabilities: Capabilities {
            output_mode: OutputMode::Envelope,
            result_schema_ref: Some(Cow::Borrowed(CALL_RESULT_SCHEMA)),
            event_schema_ref: None,
            session_schema_ref: None,
            reads_stdin: false,
            supports_output_path: false,
            requires_project: false,
            side_effects: SideEffects::Network,
            long_running: false,
            stateful: false,
        },
        capabilities_declared: true,
        exit_codes: CALL_EXIT_CODES,
    },
}];

/// The `cast` command registry. Used by `--introspect` and by adoption code
/// that needs to look up command metadata.
pub const REGISTRY: CommandRegistry = CommandRegistry::new(ENTRIES);
