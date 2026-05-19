//! Stable, agent-facing metadata for the `cast` command tree.
//!
//! See [`docs/agents/spec.md`](../../../docs/agents/spec.md). The registry
//! pins `command_id`s and machine-mode capabilities for adopted commands.

use foundry_cli::introspect::{
    CapabilityMeta, CommandMeta, CommandRegistry, OutputMode, RegistryEntry, SideEffects,
};

/// Schema id for the `cast call` envelope payload.
pub const CALL_RESULT_SCHEMA: &str = "foundry:cast.call@v1";

/// Schema id for the `cast abi-encode` envelope payload.
pub const ABI_ENCODE_RESULT_SCHEMA: &str = "foundry:cast.abi-encode@v1";

/// Schema id for the `cast abi-decode` envelope payload.
pub const ABI_DECODE_RESULT_SCHEMA: &str = "foundry:cast.abi-decode@v1";

/// Schema id for the `cast keccak` envelope payload.
pub const KECCAK_RESULT_SCHEMA: &str = "foundry:cast.keccak@v1";

/// Schema id for the `cast 4byte` envelope payload.
pub const FOUR_BYTE_RESULT_SCHEMA: &str = "foundry:cast.4byte@v1";

/// Schema id for the `cast send` envelope payload.
pub const SEND_RESULT_SCHEMA: &str = "foundry:cast.send@v1";

static ENTRIES: &[RegistryEntry] = &[
    RegistryEntry {
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
    },
    RegistryEntry {
        path: &["abi-encode"],
        meta: CommandMeta {
            command_id: Some("cast.abi-encode"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(ABI_ENCODE_RESULT_SCHEMA),
                side_effects: SideEffects::None,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["decode-abi"],
        meta: CommandMeta {
            command_id: Some("cast.abi-decode"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(ABI_DECODE_RESULT_SCHEMA),
                side_effects: SideEffects::None,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["keccak"],
        meta: CommandMeta {
            command_id: Some("cast.keccak"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(KECCAK_RESULT_SCHEMA),
                side_effects: SideEffects::None,
                reads_stdin: true,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["4byte"],
        meta: CommandMeta {
            command_id: Some("cast.4byte"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(FOUR_BYTE_RESULT_SCHEMA),
                side_effects: SideEffects::Network,
                // `--machine` requires argv; stdin is human-only.
                reads_stdin: false,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
    RegistryEntry {
        path: &["send"],
        meta: CommandMeta {
            command_id: Some("cast.send"),
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(SEND_RESULT_SCHEMA),
                side_effects: SideEffects::ChainWrite,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        },
    },
];

/// The `cast` command registry. Used by `--introspect` and by adoption code
/// that needs to look up command metadata.
pub const REGISTRY: CommandRegistry = CommandRegistry::new(ENTRIES);
