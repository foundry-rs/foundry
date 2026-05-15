//! Stable, agent-facing metadata for the `cast` command tree.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md). The registry
//! pins `command_id`s and machine-mode capabilities for adopted commands.

use foundry_cli::introspect::{
    CapabilityMeta, CommandMeta, CommandRegistry, OutputMode, RegistryEntry, SideEffects,
};

/// Stable schema id for the `cast call` envelope payload.
pub const CALL_RESULT_SCHEMA: &str = "foundry:cast.call@v1";

static ENTRIES: &[RegistryEntry] = &[RegistryEntry {
    path: &["call"],
    meta: CommandMeta {
        command_id: Some("cast.call"),
        capabilities: CapabilityMeta {
            output_mode: OutputMode::Envelope,
            result_schema_ref: Some(CALL_RESULT_SCHEMA),
            side_effects: SideEffects::Network,
            ..CapabilityMeta::NONE
        },
        exit_codes: &[],
    },
}];

/// The `cast` command registry. Used by `--introspect` and by adoption code
/// that needs to look up command metadata.
pub const REGISTRY: CommandRegistry = CommandRegistry::new(ENTRIES);
