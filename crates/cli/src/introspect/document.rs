//! Serializable types describing a Foundry binary's command surface.
//!
//! See [`docs/agents/spec.md`](../../../../docs/agents/spec.md) for the
//! contract these types implement.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Top-level introspection document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntrospectDocument {
    /// Stable logical schema id, e.g. `foundry:introspect@v1`.
    pub schema_id: String,
    /// Schema version for the introspect document.
    pub schema_version: u32,
    /// Information about the binary being introspected.
    pub binary: BinaryInfo,
    /// Tree of commands exposed by the binary.
    pub commands: Vec<CommandInfo>,
}

/// Information about the binary itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryInfo {
    /// Binary name (`forge`, `cast`, `anvil`, `chisel`).
    pub name: String,
    /// Short version string.
    pub version: String,
    /// Long version string with build metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_version: Option<String>,
    /// Description of the binary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Args accepted by every command (clap `global = true`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_args: Vec<ArgInfo>,
}

/// Information about a single command (or group) in the CLI tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInfo {
    /// Stable machine identifier (e.g. `forge.build`).
    pub command_id: String,
    /// Whether `command_id` is pinned in the per-binary registry (frozen) or
    /// derived from the clap path (provisional and may shift on CLI renames).
    pub command_id_stable: bool,
    /// Clap path components (e.g. `["forge", "build"]`).
    pub path: Vec<String>,
    /// Visible aliases for this command.
    pub aliases: Vec<String>,
    /// Short, single-line summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Long description (multi-line allowed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Arguments declared directly on this command.
    pub args: Vec<ArgInfo>,
    /// Subcommands of this command.
    pub subcommands: Vec<Self>,
    /// Capabilities reported for agent consumers.
    pub capabilities: Capabilities,
    /// Whether `capabilities` was authored in the registry. When `false`,
    /// every capability field is a non-authoritative default and consumers
    /// MUST treat side-effects, project requirement, etc. as unknown.
    pub capabilities_declared: bool,
    /// Command-specific exit codes (in addition to the global table).
    pub exit_codes: Vec<ExitCodeInfo>,
    /// Whether this command is hidden in the human-facing help.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

/// Capability flags exposed for agent consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// What the command emits when run in machine mode.
    pub output_mode: OutputMode,
    /// Stable schema id for the envelope payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_schema_ref: Option<Cow<'static, str>>,
    /// Stable schema id for stream event records.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_schema_ref: Option<Cow<'static, str>>,
    /// Stable schema id for session-record startup/state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_schema_ref: Option<Cow<'static, str>>,
    /// Whether the command can take input on stdin via `--input -`.
    pub reads_stdin: bool,
    /// Whether the command supports `--output PATH`.
    pub supports_output_path: bool,
    /// Whether the command requires a Foundry project to run.
    pub requires_project: bool,
    /// Coarse classification of the command's side effects.
    pub side_effects: SideEffects,
    /// Whether the command can stream output for an extended period.
    pub long_running: bool,
    /// Whether the command opens a session that persists beyond a single call.
    pub stateful: bool,
}

impl Capabilities {
    /// Const-constructible default suitable for use in `static` registries.
    pub const NONE: Self = Self {
        output_mode: OutputMode::None,
        result_schema_ref: None,
        event_schema_ref: None,
        session_schema_ref: None,
        reads_stdin: false,
        supports_output_path: false,
        requires_project: false,
        side_effects: SideEffects::None,
        long_running: false,
        stateful: false,
    };
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::NONE
    }
}

/// Output mode under machine mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// No machine-mode contract yet; output is human-only.
    None,
    /// Pre-existing `--json` shape predating this contract.
    LegacyJson,
    /// Single terminal `JsonEnvelope<T>` on stdout.
    Envelope,
    /// Newline-delimited JSON event records on stdout.
    Stream,
    /// Long-running session (e.g. `anvil`); emits a `session_start` record.
    Session,
}

/// Coarse classification of a command's side effects.
///
/// Reports only the highest-impact effect (e.g. a chain-writing command that
/// also writes files reports `ChainWrite`); it is not an exhaustive set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffects {
    /// Pure: reads only (e.g. `cast tx`).
    None,
    /// Writes files on the local filesystem.
    FsWrite,
    /// Performs network reads (RPC, HTTP).
    Network,
    /// Submits transactions or otherwise mutates chain state.
    ChainWrite,
    /// Spawns a long-running server (e.g. `anvil`).
    SpawnServer,
}

/// Information about a single command argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArgInfo {
    /// Argument identifier (clap arg id).
    pub name: String,
    /// Argument kind.
    pub kind: ArgKind,
    /// Best-effort classification of the value type.
    pub value_type: ValueType,
    /// Help text, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    /// Long form (`--foo`), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
    /// Short form (`-f`), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<char>,
    /// All visible aliases.
    pub aliases: Vec<String>,
    /// Bound environment variable, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// Default value, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Permitted values for value-enum arguments.
    pub possible_values: Vec<String>,
    /// Whether the argument is required.
    pub required: bool,
    /// Whether the argument can be supplied multiple times.
    pub repeatable: bool,
    /// Other arguments this argument conflicts with.
    pub conflicts_with: Vec<String>,
    /// Help heading the argument is grouped under.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_heading: Option<String>,
    /// Whether the argument is hidden in human help.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
}

/// Argument shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgKind {
    /// Boolean flag (`--quiet`).
    Flag,
    /// Option that takes a value (`--rpc-url URL`).
    Option,
    /// Positional argument.
    Positional,
}

/// Best-effort classification of an argument's value type, for agent UI hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Bool,
    String,
    Integer,
    Path,
    Url,
    Address,
    Selector,
    Hex,
    Json,
    Other,
}

/// Documented exit code for a command, beyond the global table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitCodeInfo {
    /// Numeric process exit code.
    pub code: i32,
    /// Stable name (e.g. `TestFailure`).
    pub name: Cow<'static, str>,
    /// Description of when this code is emitted.
    pub description: Cow<'static, str>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(b: &bool) -> bool {
    !*b
}
