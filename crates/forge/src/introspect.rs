//! Stable, agent-facing metadata for the `forge` command tree.
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

/// Stable schema id for the `forge build` envelope payload.
pub const BUILD_RESULT_SCHEMA: &str = "foundry:forge.build@v1";

/// Stable schema id for `forge test` stream event records.
pub const TEST_EVENT_SCHEMA: &str = "foundry:forge.test.event@v1";
/// Stable schema id for the terminal `forge test` envelope payload.
pub const TEST_RESULT_SCHEMA: &str = "foundry:forge.test@v1";

/// Exit codes `forge build` may emit under `--machine`. Declared explicitly so
/// agents don't have to assume "inherits global defaults"; codes not in this
/// list are not produced by this command.
static BUILD_EXIT_CODES: &[ExitCodeInfo] = &[
    ExitCodeInfo {
        code: ExitCode::Success.to_i32(),
        name: Cow::Borrowed(ExitCode::Success.name()),
        description: Cow::Borrowed("Build succeeded; envelope `success: true`."),
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
        code: ExitCode::Build.to_i32(),
        name: Cow::Borrowed(ExitCode::Build.name()),
        description: Cow::Borrowed(
            "Solc reported one or more compile errors (envelope `compiler.solc.error`).",
        ),
    },
];

/// Exit codes `forge test` may emit under `--machine`.
static TEST_EXIT_CODES: &[ExitCodeInfo] = &[
    ExitCodeInfo {
        code: ExitCode::Success.to_i32(),
        name: Cow::Borrowed(ExitCode::Success.name()),
        description: Cow::Borrowed(
            "All tests passed, or failures were tolerated by `--allow-failure`; envelope \
             `success: true`. When `--allow-failure` is set, `data.failed` may be non-zero.",
        ),
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
        code: ExitCode::Build.to_i32(),
        name: Cow::Borrowed(ExitCode::Build.name()),
        description: Cow::Borrowed(
            "Solc reported one or more compile errors before tests could run \
             (envelope `compiler.solc.error`).",
        ),
    },
    ExitCodeInfo {
        code: ExitCode::TestFailure.to_i32(),
        name: Cow::Borrowed(ExitCode::TestFailure.name()),
        description: Cow::Borrowed(
            "Test suite ran and at least one test failed without `--allow-failure` \
             (envelope `test.failed`).",
        ),
    },
];

static ENTRIES: &[RegistryEntry] = &[
    RegistryEntry {
        path: &["build"],
        meta: CommandMeta {
            command_id: Some("forge.build"),
            capabilities: Capabilities {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(Cow::Borrowed(BUILD_RESULT_SCHEMA)),
                event_schema_ref: None,
                session_schema_ref: None,
                reads_stdin: false,
                supports_output_path: false,
                requires_project: true,
                side_effects: SideEffects::FsWrite,
                long_running: false,
                stateful: false,
            },
            capabilities_declared: true,
            exit_codes: BUILD_EXIT_CODES,
        },
    },
    RegistryEntry {
        path: &["test"],
        meta: CommandMeta {
            command_id: Some("forge.test"),
            capabilities: Capabilities {
                output_mode: OutputMode::Stream,
                result_schema_ref: Some(Cow::Borrowed(TEST_RESULT_SCHEMA)),
                event_schema_ref: Some(Cow::Borrowed(TEST_EVENT_SCHEMA)),
                session_schema_ref: None,
                reads_stdin: false,
                supports_output_path: false,
                requires_project: true,
                // `Network` subsumes the always-present `FsWrite`; `forge
                // test` does not broadcast user txs, so not `ChainWrite`.
                side_effects: SideEffects::Network,
                long_running: true,
                stateful: false,
            },
            capabilities_declared: true,
            exit_codes: TEST_EXIT_CODES,
        },
    },
];

/// The `forge` command registry. Used by `--introspect` and by adoption code
/// that needs to look up command metadata.
pub const REGISTRY: CommandRegistry = CommandRegistry::new(ENTRIES);
