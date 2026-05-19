//! Stable, agent-facing metadata for the `forge` command tree.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md). The registry
//! pins `command_id`s and machine-mode capabilities for adopted commands.

use foundry_cli::introspect::{
    CapabilityMeta, CommandMeta, CommandRegistry, OutputMode, RegistryEntry, SideEffects,
};

/// Stable schema id for the `forge build` envelope payload.
pub const BUILD_RESULT_SCHEMA: &str = "foundry:forge.build@v1";

/// Schema id for the `forge create` envelope payload.
pub const CREATE_RESULT_SCHEMA: &str = "foundry:forge.create@v1";

/// Stable schema id for `forge test` stream event records.
pub const TEST_EVENT_SCHEMA: &str = "foundry:forge.test.event@v1";
/// Stable schema id for the terminal `forge test` envelope payload.
pub const TEST_RESULT_SCHEMA: &str = "foundry:forge.test@v1";

/// Stable schema id for `forge script` stream event records.
pub const SCRIPT_EVENT_SCHEMA: &str = "foundry:forge.script.event@v1";
/// Stable schema id for the terminal `forge script` envelope payload.
pub const SCRIPT_RESULT_SCHEMA: &str = "foundry:forge.script@v1";

static ENTRIES: &[RegistryEntry] = &[
    RegistryEntry {
        path: &["build"],
        meta: CommandMeta {
            command_id: Some("forge.build"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(BUILD_RESULT_SCHEMA),
                requires_project: true,
                side_effects: SideEffects::FsWrite,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["test"],
        meta: CommandMeta {
            command_id: Some("forge.test"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Stream,
                event_schema_ref: Some(TEST_EVENT_SCHEMA),
                result_schema_ref: Some(TEST_RESULT_SCHEMA),
                requires_project: true,
                side_effects: SideEffects::None,
                long_running: true,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["create"],
        meta: CommandMeta {
            command_id: Some("forge.create"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(CREATE_RESULT_SCHEMA),
                requires_project: true,
                side_effects: SideEffects::ChainWrite,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["script"],
        meta: CommandMeta {
            command_id: Some("forge.script"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Stream,
                event_schema_ref: Some(SCRIPT_EVENT_SCHEMA),
                result_schema_ref: Some(SCRIPT_RESULT_SCHEMA),
                requires_project: true,
                side_effects: SideEffects::ChainWrite,
                long_running: true,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
];

/// The `forge` command registry. Used by `--introspect` and by adoption code
/// that needs to look up command metadata.
pub const REGISTRY: CommandRegistry = CommandRegistry::new(ENTRIES);
