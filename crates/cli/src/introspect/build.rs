//! Build an [`IntrospectDocument`] from a `clap::Command` tree.

use clap::{Arg, ArgAction, Command};
use std::sync::OnceLock;

use super::{
    INTROSPECT_SCHEMA_ID, INTROSPECT_SCHEMA_VERSION, OutputMode,
    document::{
        ArgInfo, ArgKind, BinaryInfo, Capabilities, CommandInfo, ExitCodeInfo, IntrospectDocument,
        ValueType,
    },
    registry::{CapabilityMeta, CommandMeta, CommandRegistry, ExitCodeMeta},
};

/// Collect every `command_id` (recursively) emitted by an
/// [`IntrospectDocument`].
pub fn collect_command_ids(doc: &IntrospectDocument) -> Vec<String> {
    fn walk(cmd: &CommandInfo, out: &mut Vec<String>) {
        out.push(cmd.command_id.clone());
        for sub in &cmd.subcommands {
            walk(sub, out);
        }
    }
    let mut out = Vec::new();
    for cmd in &doc.commands {
        walk(cmd, &mut out);
    }
    out
}

/// Assert capability self-consistency for every command in `doc`.
///
/// Returns one error message per offending command. Static repo-wide check
/// that catches commands declaring an output mode without wiring the
/// supporting schema metadata, or vice versa.
///
/// Per-mode rules (see also spec §3, §4, §8):
///
/// - [`OutputMode::None`](super::OutputMode::None) and
///   [`OutputMode::LegacyJson`](super::OutputMode::LegacyJson) MUST NOT carry schema refs.
/// - [`OutputMode::Envelope`](super::OutputMode::Envelope) requires `result_schema_ref`; MUST NOT
///   carry `event_schema_ref` or `session_schema_ref`.
/// - [`OutputMode::Stream`](super::OutputMode::Stream) requires `event_schema_ref`; implies
///   `long_running = true`. `result_schema_ref` MAY also be set when the stream ends with a
///   terminal envelope.
/// - [`OutputMode::Session`](super::OutputMode::Session) requires `session_schema_ref`; implies
///   `stateful = true` and `long_running = true`.
///
/// Schema refs, when present, must be non-empty and match
/// `^foundry:[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*(\.event|\.session)?@v\d+$`.
pub fn capability_violations(doc: &IntrospectDocument) -> Vec<String> {
    static SCHEMA_REF_RE: OnceLock<regex::Regex> = OnceLock::new();
    let schema_re = SCHEMA_REF_RE.get_or_init(|| {
        regex::Regex::new(r"^foundry:[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*(\.event|\.session)?@v\d+$")
            .expect("schema-ref regex compiles")
    });

    fn check_ref(
        out: &mut Vec<String>,
        id: &str,
        name: &str,
        val: Option<&str>,
        re: &regex::Regex,
    ) {
        if let Some(v) = val {
            if v.is_empty() {
                out.push(format!("{id}: capabilities.{name} must not be empty"));
            } else if !re.is_match(v) {
                out.push(format!(
                    "{id}: capabilities.{name} = `{v}` does not match `foundry:<id>@vN`"
                ));
            }
        }
    }

    fn walk(cmd: &CommandInfo, out: &mut Vec<String>, re: &regex::Regex) {
        let caps = &cmd.capabilities;
        let id = cmd.command_id.as_str();

        // Format check on every present ref, regardless of mode.
        check_ref(out, id, "result_schema_ref", caps.result_schema_ref.as_deref(), re);
        check_ref(out, id, "event_schema_ref", caps.event_schema_ref.as_deref(), re);
        check_ref(out, id, "session_schema_ref", caps.session_schema_ref.as_deref(), re);

        match caps.output_mode {
            OutputMode::None | OutputMode::LegacyJson => {
                if caps.result_schema_ref.is_some()
                    || caps.event_schema_ref.is_some()
                    || caps.session_schema_ref.is_some()
                {
                    out.push(format!(
                        "{id}: output_mode={:?} must not carry any schema refs",
                        caps.output_mode
                    ));
                }
            }
            OutputMode::Envelope => {
                if caps.result_schema_ref.is_none() {
                    out.push(format!(
                        "{id}: output_mode=envelope requires capabilities.result_schema_ref"
                    ));
                }
                if caps.event_schema_ref.is_some() {
                    out.push(format!("{id}: output_mode=envelope must not carry event_schema_ref"));
                }
                if caps.session_schema_ref.is_some() {
                    out.push(format!(
                        "{id}: output_mode=envelope must not carry session_schema_ref"
                    ));
                }
            }
            OutputMode::Stream => {
                if caps.event_schema_ref.is_none() {
                    out.push(format!(
                        "{id}: output_mode=stream requires capabilities.event_schema_ref"
                    ));
                }
                if !caps.long_running {
                    out.push(format!("{id}: output_mode=stream implies long_running = true"));
                }
            }
            OutputMode::Session => {
                if caps.session_schema_ref.is_none() {
                    out.push(format!(
                        "{id}: output_mode=session requires capabilities.session_schema_ref"
                    ));
                }
                if !caps.stateful {
                    out.push(format!("{id}: output_mode=session implies stateful = true"));
                }
                if !caps.long_running {
                    out.push(format!("{id}: output_mode=session implies long_running = true"));
                }
            }
        }

        for sub in &cmd.subcommands {
            walk(sub, out, re);
        }
    }

    let mut out = Vec::new();
    for cmd in &doc.commands {
        walk(cmd, &mut out, schema_re);
    }
    out
}

/// Assert that every `command_id` in `doc` is unique.
///
/// Returns the duplicate ids on failure (one entry per duplicate `command_id`,
/// in the order they were first encountered). On success returns an empty vec.
///
/// This is the canonical uniqueness check the agent contract relies on; each
/// binary calls it from a unit test to enforce the invariant in CI.
pub fn duplicate_command_ids(doc: &IntrospectDocument) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut dups = Vec::new();
    for id in collect_command_ids(doc) {
        if !seen.insert(id.clone()) && !dups.contains(&id) {
            dups.push(id);
        }
    }
    dups
}

/// Build an [`IntrospectDocument`] for `command` overlaid with metadata from
/// `registry`.
pub fn build_document(command: &Command, registry: &CommandRegistry) -> IntrospectDocument {
    let binary = build_binary_info(command);
    let mut commands = Vec::new();

    let root_path = vec![command.get_name().to_string()];
    for sub in command.get_subcommands() {
        commands.push(build_command_info(sub, &root_path, registry));
    }

    IntrospectDocument {
        schema_id: INTROSPECT_SCHEMA_ID.to_string(),
        schema_version: INTROSPECT_SCHEMA_VERSION,
        binary,
        commands,
    }
}

fn build_binary_info(command: &Command) -> BinaryInfo {
    BinaryInfo {
        name: command.get_name().to_string(),
        version: command.get_version().unwrap_or("").to_string(),
        long_version: command.get_long_version().map(str::to_string),
        description: command.get_about().map(|s| s.to_string()),
    }
}

fn build_command_info(
    command: &Command,
    parent_path: &[String],
    registry: &CommandRegistry,
) -> CommandInfo {
    // Path components for this command, including the binary name at index 0.
    let mut path = parent_path.to_vec();
    path.push(command.get_name().to_string());

    // Registry lookup uses the path **without** the binary name.
    let lookup_path: Vec<&str> = path.iter().skip(1).map(String::as_str).collect();
    let meta = registry.lookup(&lookup_path);

    let command_id = derive_command_id(&path, meta);
    let capabilities =
        meta.map_or_else(Capabilities::default, |m| capabilities_from(&m.capabilities));
    let exit_codes =
        meta.map_or_else(Vec::new, |m| m.exit_codes.iter().map(exit_code_from).collect());

    let aliases = command.get_visible_aliases().map(str::to_string).collect::<Vec<_>>();

    let summary = command.get_about().map(|s| s.to_string());
    let description = command
        .get_long_about()
        .map(|s| s.to_string())
        .filter(|d| Some(d.as_str()) != summary.as_deref());

    let args =
        command.get_arguments().filter(|a| !is_help_or_version(a)).map(build_arg_info).collect();

    let subcommands =
        command.get_subcommands().map(|sub| build_command_info(sub, &path, registry)).collect();

    CommandInfo {
        command_id,
        path,
        aliases,
        summary,
        description,
        args,
        subcommands,
        capabilities,
        exit_codes,
        hidden: command.is_hide_set(),
    }
}

/// Convert const-friendly registry [`CapabilityMeta`] to the owned
/// serialized [`Capabilities`] form emitted in introspection.
fn capabilities_from(meta: &CapabilityMeta) -> Capabilities {
    Capabilities {
        output_mode: meta.output_mode,
        result_schema_ref: meta.result_schema_ref.map(String::from),
        event_schema_ref: meta.event_schema_ref.map(String::from),
        session_schema_ref: meta.session_schema_ref.map(String::from),
        reads_stdin: meta.reads_stdin,
        supports_output_path: meta.supports_output_path,
        requires_project: meta.requires_project,
        side_effects: meta.side_effects,
        long_running: meta.long_running,
        stateful: meta.stateful,
    }
}

/// Convert const-friendly registry [`ExitCodeMeta`] to the owned serialized
/// [`ExitCodeInfo`] form emitted in introspection.
fn exit_code_from(meta: &ExitCodeMeta) -> ExitCodeInfo {
    ExitCodeInfo {
        code: meta.code,
        name: meta.name.to_string(),
        description: meta.description.to_string(),
    }
}

/// Derive the stable command id.
///
/// If the registry pins an explicit `command_id`, use it. Otherwise, derive
/// the id by joining the path components with `.` (e.g. `forge.build`).
fn derive_command_id(path: &[String], meta: Option<&CommandMeta>) -> String {
    if let Some(id) = meta.and_then(|m| m.command_id) {
        return id.to_string();
    }
    path.join(".")
}

fn build_arg_info(arg: &Arg) -> ArgInfo {
    let kind = arg_kind(arg);
    let value_type = arg_value_type(arg);

    let aliases = arg
        .get_visible_aliases()
        .map(|a| a.into_iter().map(String::from).collect::<Vec<_>>())
        .unwrap_or_default();

    let possible_values = arg
        .get_possible_values()
        .iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_string())
        .collect();

    // Clap does not expose conflict relationships through a public API on `Arg`;
    // these would have to be threaded through annotations on a per-binary basis.
    // Reserved here so the schema field is always present and stable.
    let conflicts_with: Vec<String> = Vec::new();

    let default = arg.get_default_values().first().map(|v| v.to_string_lossy().into_owned());

    ArgInfo {
        name: arg.get_id().to_string(),
        kind,
        value_type,
        help: arg.get_help().map(|h| h.to_string()),
        long: arg.get_long().map(String::from),
        short: arg.get_short(),
        aliases,
        env: arg.get_env().map(|e| e.to_string_lossy().into_owned()),
        default,
        possible_values,
        required: arg.is_required_set(),
        repeatable: matches!(arg.get_action(), ArgAction::Count | ArgAction::Append),
        conflicts_with,
        help_heading: arg.get_help_heading().map(String::from),
        hidden: arg.is_hide_set(),
    }
}

fn arg_kind(arg: &Arg) -> ArgKind {
    if arg.is_positional() {
        return ArgKind::Positional;
    }
    match arg.get_action() {
        ArgAction::SetTrue
        | ArgAction::SetFalse
        | ArgAction::Count
        | ArgAction::Help
        | ArgAction::HelpShort
        | ArgAction::HelpLong
        | ArgAction::Version => ArgKind::Flag,
        _ => ArgKind::Option,
    }
}

fn arg_value_type(arg: &Arg) -> ValueType {
    match arg.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse => return ValueType::Bool,
        ArgAction::Count => return ValueType::Integer,
        _ => {}
    }

    let name = arg.get_value_names().and_then(|v| v.first()).map(|s| s.as_str()).unwrap_or("");
    match name.to_ascii_lowercase().as_str() {
        "" => ValueType::Other,
        "path" | "file" | "dir" | "directory" => ValueType::Path,
        "url" | "rpc_url" | "rpc-url" => ValueType::Url,
        "address" | "addr" => ValueType::Address,
        "selector" | "sig" => ValueType::Selector,
        "hex" | "bytes" | "bytecode" | "calldata" => ValueType::Hex,
        "json" => ValueType::Json,
        "int" | "integer" | "u64" | "u256" | "i64" | "i256" | "number" | "n" => ValueType::Integer,
        "string" | "str" | "name" => ValueType::String,
        _ => ValueType::Other,
    }
}

fn is_help_or_version(arg: &Arg) -> bool {
    matches!(
        arg.get_action(),
        ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    #[command(name = "demo", version = "0.1.0")]
    struct Demo {
        #[command(subcommand)]
        cmd: DemoSub,
    }

    #[derive(clap::Subcommand)]
    enum DemoSub {
        /// Build the project.
        #[command(visible_alias = "b")]
        Build {
            /// Number of jobs.
            #[arg(short, long)]
            jobs: Option<u32>,
        },
        /// Group of cache subcommands.
        Cache {
            #[command(subcommand)]
            cmd: CacheSub,
        },
    }

    #[derive(clap::Subcommand)]
    enum CacheSub {
        /// Clean cache.
        Clean,
    }

    #[test]
    fn builds_document_with_default_command_ids() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);

        assert_eq!(doc.schema_id, INTROSPECT_SCHEMA_ID);
        assert_eq!(doc.binary.name, "demo");

        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();
        assert_eq!(build.command_id, "demo.build");
        assert_eq!(build.aliases, vec!["b".to_string()]);
    }

    #[test]
    fn registry_overrides_command_id() {
        static ENTRIES: &[super::super::registry::RegistryEntry] =
            &[super::super::registry::RegistryEntry {
                path: &["build"],
                meta: CommandMeta {
                    command_id: Some("demo.compile"),
                    capabilities: CapabilityMeta::NONE,
                    exit_codes: &[],
                },
            }];
        let registry = CommandRegistry::new(ENTRIES);

        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);

        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();
        assert_eq!(build.command_id, "demo.compile");
    }

    #[test]
    fn nested_subcommands_have_dotted_ids() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);

        let cache = doc.commands.iter().find(|c| c.path.last().unwrap() == "cache").unwrap();
        let clean = cache.subcommands.iter().find(|c| c.path.last().unwrap() == "clean").unwrap();
        assert_eq!(clean.command_id, "demo.cache.clean");
        assert_eq!(clean.path, vec!["demo", "cache", "clean"]);
    }

    #[test]
    fn args_are_described() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();

        let jobs = build.args.iter().find(|a| a.name == "jobs").unwrap();
        assert!(matches!(jobs.kind, ArgKind::Option));
        assert_eq!(jobs.long.as_deref(), Some("jobs"));
        assert_eq!(jobs.short, Some('j'));
        assert!(!jobs.required);
    }

    #[test]
    fn help_and_version_args_are_excluded() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();
        for arg in &build.args {
            assert!(arg.name != "help" && arg.name != "version", "got {arg:?}");
        }
    }

    /// Helper: build a registry with one entry pinned at `["build"]`.
    fn registry_with_one_build_entry(meta: &'static CommandMeta) -> CommandRegistry {
        let entry: &'static super::super::registry::RegistryEntry =
            Box::leak(Box::new(super::super::registry::RegistryEntry {
                path: &["build"],
                meta: *meta,
            }));
        CommandRegistry::new(std::slice::from_ref(entry))
    }

    #[test]
    fn capability_violations_detects_envelope_without_schema_ref() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        static ENTRIES: &[super::super::registry::RegistryEntry] =
            &[super::super::registry::RegistryEntry { path: &["build"], meta: META }];
        let registry = CommandRegistry::new(ENTRIES);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let violations = capability_violations(&doc);
        assert_eq!(violations.len(), 1, "got {violations:?}");
        assert!(violations[0].contains("envelope requires capabilities.result_schema_ref"));
    }

    #[test]
    fn capability_violations_passes_for_well_formed_envelope() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some("foundry:demo.build@v1"),
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        static ENTRIES: &[super::super::registry::RegistryEntry] =
            &[super::super::registry::RegistryEntry { path: &["build"], meta: META }];
        let registry = CommandRegistry::new(ENTRIES);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let violations = capability_violations(&doc);
        assert!(violations.is_empty(), "unexpected violations: {violations:?}");
    }

    #[test]
    fn capability_violations_rejects_empty_schema_ref() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some(""),
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        let registry = registry_with_one_build_entry(&META);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let v = capability_violations(&doc);
        assert!(v.iter().any(|s| s.contains("must not be empty")), "got {v:?}");
    }

    #[test]
    fn capability_violations_rejects_malformed_schema_ref() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some("not-a-foundry-ref"),
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        let registry = registry_with_one_build_entry(&META);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let v = capability_violations(&doc);
        assert!(v.iter().any(|s| s.contains("does not match")), "got {v:?}");
    }

    #[test]
    fn capability_violations_rejects_envelope_with_event_ref() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Envelope,
                result_schema_ref: Some("foundry:demo.build@v1"),
                event_schema_ref: Some("foundry:demo.build.event@v1"),
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        let registry = registry_with_one_build_entry(&META);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let v = capability_violations(&doc);
        assert!(v.iter().any(|s| s.contains("must not carry event_schema_ref")), "got {v:?}");
    }

    #[test]
    fn capability_violations_rejects_none_with_schema_ref() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::None,
                result_schema_ref: Some("foundry:demo.build@v1"),
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        let registry = registry_with_one_build_entry(&META);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let v = capability_violations(&doc);
        assert!(v.iter().any(|s| s.contains("must not carry any schema refs")), "got {v:?}");
    }

    #[test]
    fn capability_violations_session_requires_stateful() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Session,
                session_schema_ref: Some("foundry:demo.session@v1"),
                long_running: true,
                stateful: false,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        let registry = registry_with_one_build_entry(&META);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let v = capability_violations(&doc);
        assert!(v.iter().any(|s| s.contains("implies stateful")), "got {v:?}");
    }

    #[test]
    fn capability_violations_stream_requires_long_running() {
        static META: CommandMeta = CommandMeta {
            command_id: None,
            capabilities: CapabilityMeta {
                output_mode: OutputMode::Stream,
                event_schema_ref: Some("foundry:demo.event@v1"),
                long_running: false,
                ..CapabilityMeta::NONE
            },
            exit_codes: &[],
        };
        let registry = registry_with_one_build_entry(&META);
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);
        let v = capability_violations(&doc);
        assert!(v.iter().any(|s| s.contains("implies long_running")), "got {v:?}");
    }

    #[test]
    fn document_round_trips_through_serde() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let json = serde_json::to_string(&doc).unwrap();
        let parsed: IntrospectDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, doc);
    }
}
