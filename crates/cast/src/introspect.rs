//! Stable, agent-facing metadata for the `cast` command tree.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md). The registry
//! pins `command_id`s and machine-mode capabilities for adopted commands.

use foundry_cli::introspect::{
    Capabilities, CommandMeta, CommandRegistry, OutputMode, RegistryEntry, SideEffects,
};
use std::borrow::Cow;

/// Stable schema id for the `cast call` envelope payload.
pub const CALL_RESULT_SCHEMA: &str = "foundry:cast.call@v1";

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
        exit_codes: &[],
    },
}];

/// The `cast` command registry. Used by `--introspect` and by adoption code
/// that needs to look up command metadata.
pub const REGISTRY: CommandRegistry = CommandRegistry::new(ENTRIES);
