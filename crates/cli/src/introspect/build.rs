//! Build an [`IntrospectDocument`] from a `clap::Command` tree.

use clap::{Arg, ArgAction, Command};

use super::{
    INTROSPECT_SCHEMA_ID, INTROSPECT_SCHEMA_VERSION,
    document::{
        ArgInfo, ArgKind, BinaryInfo, Capabilities, CommandInfo, IntrospectDocument, ValueType,
    },
    registry::{CommandMeta, CommandRegistry},
};

impl CommandInfo {
    /// Push this command's id and every descendant id into `out`.
    fn collect_ids_into(&self, out: &mut Vec<String>) {
        out.push(self.command_id.clone());
        for sub in &self.subcommands {
            sub.collect_ids_into(out);
        }
    }
}

/// Collect every `command_id` (recursively) emitted by an
/// [`IntrospectDocument`].
pub fn collect_command_ids(doc: &IntrospectDocument) -> Vec<String> {
    let mut out = Vec::new();
    for cmd in &doc.commands {
        cmd.collect_ids_into(&mut out);
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

    // Surface the root/default invocation when the binary accepts root-only
    // (non-global) args without requiring a subcommand.
    if let Some(root) = build_root_command_info(command, registry) {
        commands.push(root);
    }

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
    // Root `global = true` args; not surfaced by `get_arguments()` on subcommands.
    let global_args = command
        .get_arguments()
        .filter(|a| a.is_global_set() && !is_help_or_version(a))
        .map(build_arg_info)
        .collect();

    BinaryInfo {
        name: command.get_name().to_string(),
        version: command.get_version().unwrap_or("").to_string(),
        long_version: command.get_long_version().map(str::to_string),
        description: command.get_about().map(|s| s.to_string()),
        global_args,
    }
}

/// Build a synthetic `CommandInfo` for the root/default invocation.
///
/// Returns `None` when the root requires a subcommand or has no root-only args
/// (i.e. nothing to invoke without a subcommand). The returned command has an
/// empty `subcommands` list; named subcommands remain top-level siblings.
///
/// Registry lookup uses the empty path (`&[]`), so a binary can pin a stable
/// id and capabilities for its default invocation (e.g. `anvil.start`).
fn build_root_command_info(command: &Command, registry: &CommandRegistry) -> Option<CommandInfo> {
    if command.is_subcommand_required_set() {
        return None;
    }

    let args: Vec<ArgInfo> = command
        .get_arguments()
        .filter(|a| !a.is_global_set() && !is_help_or_version(a))
        .map(build_arg_info)
        .collect();

    if args.is_empty() {
        return None;
    }

    let path = vec![command.get_name().to_string()];
    let meta = registry.lookup(&[]);

    let command_id = derive_command_id(&path, meta);
    let command_id_stable = meta.and_then(|m| m.command_id).is_some();
    let capabilities = meta.map_or_else(Capabilities::default, |m| m.capabilities.clone());
    let capabilities_declared = meta.is_some_and(|m| m.capabilities_declared);
    let exit_codes = meta.map_or_else(Vec::new, |m| m.exit_codes.to_vec());

    let aliases = command.get_visible_aliases().map(str::to_string).collect::<Vec<_>>();
    let summary = command.get_about().map(|s| s.to_string());
    let description = command
        .get_long_about()
        .map(|s| s.to_string())
        .filter(|d| Some(d.as_str()) != summary.as_deref());

    Some(CommandInfo {
        command_id,
        command_id_stable,
        path,
        aliases,
        summary,
        description,
        args,
        subcommands: Vec::new(),
        capabilities,
        capabilities_declared,
        exit_codes,
        hidden: command.is_hide_set(),
    })
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
    let command_id_stable = meta.and_then(|m| m.command_id).is_some();
    let capabilities = meta.map_or_else(Capabilities::default, |m| m.capabilities.clone());
    let capabilities_declared = meta.is_some_and(|m| m.capabilities_declared);
    let exit_codes = meta.map_or_else(Vec::new, |m| m.exit_codes.to_vec());

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
        command_id_stable,
        path,
        aliases,
        summary,
        description,
        args,
        subcommands,
        capabilities,
        capabilities_declared,
        exit_codes,
        hidden: command.is_hide_set(),
    }
}

/// Serialize an [`IntrospectDocument`] as compact JSON.
///
/// This is the pure rendering step `--introspect` performs before exit, split
/// out so binaries and tests can validate the emitted JSON without spawning
/// a subprocess.
pub fn render_introspect_document(command: &Command, registry: &CommandRegistry) -> String {
    let doc = build_document(command, registry);
    serde_json::to_string(&doc).expect("introspect document must be serializable")
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
        /// Global flag, must appear on `BinaryInfo.global_args`.
        #[arg(global = true, long)]
        quiet: bool,

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
    fn global_args_land_on_binary_info() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);

        assert!(
            doc.binary.global_args.iter().any(|a| a.name == "quiet"),
            "global args missing from BinaryInfo: {:?}",
            doc.binary.global_args,
        );
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
                    capabilities: Capabilities::NONE,
                    capabilities_declared: true,
                    exit_codes: &[],
                },
            }];
        let registry = CommandRegistry::new(ENTRIES);

        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);

        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();
        assert_eq!(build.command_id, "demo.compile");
        // Pinned in the registry → stable; declared capabilities → authoritative.
        assert!(build.command_id_stable);
        assert!(build.capabilities_declared);
    }

    #[test]
    fn partial_registry_entry_does_not_promote_default_capabilities() {
        // A registry entry that pins only `command_id` MUST NOT flip
        // `capabilities_declared` to true; the wire field still reflects the
        // placeholder `Capabilities::NONE` as non-authoritative.
        static ENTRIES: &[super::super::registry::RegistryEntry] =
            &[super::super::registry::RegistryEntry {
                path: &["build"],
                meta: CommandMeta {
                    command_id: Some("demo.compile"),
                    capabilities: Capabilities::NONE,
                    capabilities_declared: false,
                    exit_codes: &[],
                },
            }];
        let registry = CommandRegistry::new(ENTRIES);

        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);

        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();
        assert!(build.command_id_stable);
        assert!(!build.capabilities_declared);
    }

    #[test]
    fn unregistered_commands_are_provisional() {
        // Without a registry entry, both provenance bits must be false so
        // consumers know not to treat the defaults as authoritative.
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let build = doc.commands.iter().find(|c| c.path.last().unwrap() == "build").unwrap();
        assert!(!build.command_id_stable);
        assert!(!build.capabilities_declared);
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

    #[test]
    fn document_round_trips_through_serde() {
        let cmd = <Demo as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let json = serde_json::to_string(&doc).unwrap();
        let parsed: IntrospectDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn schema_id_and_version_agree() {
        // The `@vN` suffix on `schema_id` must match the numeric `schema_version`.
        let expected = format!("foundry:introspect@v{INTROSPECT_SCHEMA_VERSION}");
        assert_eq!(INTROSPECT_SCHEMA_ID, expected);
    }

    #[derive(Parser)]
    #[command(name = "rooted", version = "0.1.0")]
    struct Rooted {
        /// Global flag, must land on `BinaryInfo.global_args`.
        #[arg(global = true, long)]
        quiet: bool,

        /// Root-only arg, must land on the synthetic root `CommandInfo`.
        #[arg(long)]
        port: Option<u16>,

        #[command(subcommand)]
        cmd: Option<RootedSub>,
    }

    #[derive(clap::Subcommand)]
    enum RootedSub {
        /// Named subcommand, stays a top-level sibling of the root command.
        Serve,
    }

    #[test]
    fn root_non_global_args_are_emitted_as_synthetic_root_command() {
        let cmd = <Rooted as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);

        // Global stays on `BinaryInfo.global_args`.
        assert!(doc.binary.global_args.iter().any(|a| a.name == "quiet"));

        // Synthetic root command exists with `path = [binary_name]`.
        let root = doc
            .commands
            .iter()
            .find(|c| c.path == ["rooted"])
            .expect("root command_info must be present");

        // Root-only arg lives on the root command.
        assert!(root.args.iter().any(|a| a.name == "port"));
        // Global is not duplicated onto the root command.
        assert!(!root.args.iter().any(|a| a.name == "quiet"));
        // Named subcommands are siblings, not nested under the root command.
        assert!(root.subcommands.is_empty());
        assert!(doc.commands.iter().any(|c| c.path == ["rooted", "serve"]));
    }

    #[test]
    fn root_command_can_be_overridden_by_empty_registry_path() {
        static ENTRIES: &[super::super::registry::RegistryEntry] =
            &[super::super::registry::RegistryEntry {
                path: &[],
                meta: CommandMeta {
                    command_id: Some("rooted.start"),
                    capabilities: Capabilities::NONE,
                    capabilities_declared: true,
                    exit_codes: &[],
                },
            }];
        let registry = CommandRegistry::new(ENTRIES);

        let cmd = <Rooted as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &registry);

        let root = doc.commands.iter().find(|c| c.path == ["rooted"]).unwrap();
        assert_eq!(root.command_id, "rooted.start");
        assert!(root.command_id_stable);
        assert!(root.capabilities_declared);
    }

    #[test]
    fn subcommand_required_root_does_not_emit_synthetic_root_command() {
        let cmd = clap::Command::new("strict")
            .subcommand_required(true)
            .arg(clap::Arg::new("port").long("port"))
            .subcommand(clap::Command::new("run"));
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);

        // Only the named subcommand is emitted; no synthetic root entry.
        assert!(!doc.commands.iter().any(|c| c.path == ["strict"]));
        assert!(doc.commands.iter().any(|c| c.path == ["strict", "run"]));
    }

    #[test]
    fn build_does_not_require_successful_parse() {
        // `--introspect` must work even when required args/subcommands are
        // missing; `build_document` is the only function called pre-parse, so
        // it must not invoke clap's parsing path.
        let cmd = clap::Command::new("strict")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(clap::Command::new("run").arg(clap::Arg::new("input").required(true)));
        let json = render_introspect_document(&cmd, &CommandRegistry::EMPTY);
        let parsed: IntrospectDocument = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0].command_id, "strict.run");
    }
}
