use crate::{
    cmd::{cache::CacheSubcommands, generate::GenerateSubcommands, test::TestSummaryData, watch},
    introspect::REGISTRY,
    opts::{Forge, ForgeSubcommand},
    result::TestOutcome,
};
use clap::CommandFactory;
use clap_complete::generate;
use eyre::Result;
use foundry_cli::{
    json::{JsonEnvelope, JsonMessage, print_json},
    utils,
};
use foundry_common::{sh_warn, shell};
use foundry_evm::inspectors::cheatcodes::{ForgeContext, set_execution_context};

/// Run the `forge` command line interface.
pub fn run() -> Result<()> {
    // Pre-setup so setup failures land in the machine envelope path.
    foundry_cli::machine::check_machine();
    setup()?;

    foundry_cli::opts::GlobalArgs::check_introspect_with(Forge::command(), &REGISTRY);
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Forge>();

    let args = foundry_cli::parse_or_exit::<Forge>();
    args.global.init()?;

    run_command(args)
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<()> {
    utils::common_setup();
    utils::subscriber();

    Ok(())
}

/// Run the subcommand.
pub fn run_command(args: Forge) -> Result<()> {
    // Set the execution context based on the subcommand.
    let context = match &args.cmd {
        ForgeSubcommand::Test(_) => ForgeContext::Test,
        ForgeSubcommand::Coverage(_) => ForgeContext::Coverage,
        ForgeSubcommand::Snapshot(_) => ForgeContext::Snapshot,
        ForgeSubcommand::Script(cmd) => {
            if cmd.broadcast {
                ForgeContext::ScriptBroadcast
            } else if cmd.resume {
                ForgeContext::ScriptResume
            } else {
                ForgeContext::ScriptDryRun
            }
        }
        _ => ForgeContext::Unknown,
    };
    set_execution_context(context);

    let global = &args.global;

    // Run the subcommand.
    match args.cmd {
        ForgeSubcommand::Test(cmd) => {
            // Preflight runs before the watcher dispatch so `--watch` (and
            // every other unsupported flag) is rejected at the top level
            // rather than swallowed by the watch loop.
            cmd.reject_machine_unsupported_flags()?;
            if cmd.is_watch() {
                global.block_on(watch::watch_test(cmd))
            } else {
                let machine_mode = foundry_cli::is_machine();
                let silent = machine_mode || cmd.junit || shell::is_json();
                let started = std::time::Instant::now();
                let outcome = global.block_on(cmd.run())?;
                if machine_mode {
                    return finalize_test_machine_mode(outcome, started.elapsed());
                }
                outcome.ensure_ok(silent)
            }
        }
        ForgeSubcommand::Script(cmd) => global.block_on(cmd.run_script()),
        ForgeSubcommand::Coverage(cmd) => {
            if cmd.is_watch() {
                global.block_on(watch::watch_coverage(cmd))
            } else {
                global.block_on(cmd.run())
            }
        }
        ForgeSubcommand::Bind(cmd) => cmd.run(),
        ForgeSubcommand::Build(cmd) => {
            if cmd.is_watch() {
                global.block_on(watch::watch_build(cmd))
            } else {
                global.block_on(cmd.run()).map(drop)
            }
        }
        ForgeSubcommand::VerifyContract(args) => global.block_on(args.run()),
        ForgeSubcommand::VerifyCheck(args) => global.block_on(args.run()),
        ForgeSubcommand::VerifyBytecode(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Clone(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Cache(cmd) => match cmd.sub {
            CacheSubcommands::Clean(cmd) => cmd.run(),
            CacheSubcommands::Ls(cmd) => cmd.run(),
        },
        ForgeSubcommand::Create(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Update(cmd) => cmd.run(),
        ForgeSubcommand::Install(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Remove(cmd) => cmd.run(),
        ForgeSubcommand::Remappings(cmd) => cmd.run(),
        ForgeSubcommand::Init(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Completions { shell } => {
            generate(shell, &mut Forge::command(), "forge", &mut std::io::stdout());
            Ok(())
        }
        ForgeSubcommand::Clean { root } => {
            let config = utils::load_config_with_root(root.as_deref())?;
            let project = config.project()?;
            for warning in config.cleanup(&project)? {
                let _ = sh_warn!("{warning}");
            }
            Ok(())
        }
        ForgeSubcommand::Snapshot(cmd) => {
            if cmd.is_watch() {
                global.block_on(watch::watch_gas_snapshot(cmd))
            } else {
                global.block_on(cmd.run())
            }
        }
        ForgeSubcommand::Fmt(cmd) => {
            if cmd.is_watch() {
                global.block_on(watch::watch_fmt(cmd))
            } else {
                cmd.run()
            }
        }
        ForgeSubcommand::Config(cmd) => cmd.run(),
        ForgeSubcommand::Flatten(cmd) => cmd.run(),
        ForgeSubcommand::Inspect(cmd) => cmd.run(),
        ForgeSubcommand::Tree(cmd) => cmd.run(),
        ForgeSubcommand::Geiger(cmd) => cmd.run(),
        ForgeSubcommand::Doc(cmd) => {
            if cmd.is_watch() {
                global.block_on(watch::watch_doc(cmd))
            } else {
                global.block_on(cmd.run())?;
                Ok(())
            }
        }
        ForgeSubcommand::Selectors { command } => global.block_on(command.run()),
        ForgeSubcommand::Generate(cmd) => match cmd.sub {
            GenerateSubcommands::Test(cmd) => cmd.run(),
        },
        ForgeSubcommand::Compiler(cmd) => cmd.run(),
        ForgeSubcommand::Soldeer(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Eip712(cmd) => cmd.run(),
        ForgeSubcommand::BindJson(cmd) => cmd.run(),
        ForgeSubcommand::Lint(cmd) => cmd.run(),
    }
}

/// Emit the terminal `forge test` envelope and exit appropriately under
/// `--machine`. Bypasses [`TestOutcome::ensure_ok`]'s human output entirely;
/// the agent stream owns stdout for the run.
fn finalize_test_machine_mode(outcome: TestOutcome, wall_clock: std::time::Duration) -> Result<()> {
    let summary = TestSummaryData::from_outcome(&outcome, wall_clock);
    let warnings = aggregate_test_warnings(&outcome);

    if outcome.allow_failure || outcome.failed() == 0 {
        print_json(&JsonEnvelope::success_with_warnings(summary, warnings))?;
        return Ok(());
    }
    let details = serde_json::to_value(&summary).unwrap_or(serde_json::Value::Null);
    let message =
        format!("{} test(s) failed across {} suite(s)", outcome.failed(), outcome.results.len());
    let mut envelope = JsonEnvelope::error(
        JsonMessage::error(foundry_cli::diagnostic::test::FAILED, message).with_details(details),
    );
    envelope.warnings = warnings;
    print_json(&envelope)?;
    std::process::exit(foundry_cli::ExitCode::TestFailure.to_i32());
}

/// Collect every `SuiteResult.warnings` string into structured envelope
/// messages keyed by the stable `test.warning` code.
fn aggregate_test_warnings(outcome: &TestOutcome) -> Vec<JsonMessage> {
    outcome
        .results
        .iter()
        .flat_map(|(suite, sr)| {
            sr.warnings.iter().map(move |w| {
                JsonMessage::warning(foundry_cli::diagnostic::test::WARNING, w.clone())
                    .with_details(serde_json::json!({ "suite": suite }))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_cli::introspect::{
        OutputMode, build_document, capability_violations, duplicate_command_ids,
    };

    /// Every `command_id` exposed by `forge --introspect` MUST be unique.
    /// This is the foundation of the agent contract — agents key on
    /// `command_id` to identify commands, and duplicates would silently break
    /// downstream tooling.
    #[test]
    fn introspect_command_ids_are_unique() {
        let cmd = <Forge as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &REGISTRY);
        let dups = duplicate_command_ids(&doc);
        assert!(dups.is_empty(), "duplicate forge command_ids: {dups:?}");
    }

    /// Capability self-consistency: any command declaring an output mode
    /// must wire the matching schema reference. See
    /// [`capability_violations`].
    #[test]
    fn introspect_capabilities_are_consistent() {
        let cmd = <Forge as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &REGISTRY);
        let v = capability_violations(&doc);
        assert!(v.is_empty(), "forge capability violations: {v:?}");
    }

    /// Every `*_schema_ref` exposed by `forge --introspect` MUST point at a
    /// committed JSON Schema file under `docs/agents/schemas/`. Guards
    /// against typos and ref bumps that would leave agent tooling with
    /// dangling pointers.
    #[test]
    fn introspect_schema_refs_resolve_to_committed_schemas() {
        use foundry_test_utils::agent_schema;
        let known: std::collections::BTreeSet<&'static str> =
            agent_schema::known_schema_ids().collect();
        let cmd = <Forge as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &REGISTRY);

        fn walk(
            c: &foundry_cli::introspect::CommandInfo,
            known: &std::collections::BTreeSet<&'static str>,
            errs: &mut Vec<String>,
        ) {
            let caps = &c.capabilities;
            for (name, v) in [
                ("result_schema_ref", caps.result_schema_ref.as_deref()),
                ("event_schema_ref", caps.event_schema_ref.as_deref()),
                ("session_schema_ref", caps.session_schema_ref.as_deref()),
            ] {
                if let Some(id) = v
                    && !known.contains(id)
                {
                    errs.push(format!("{}: {name} points to unknown schema `{id}`", c.command_id));
                }
            }
            for sub in &c.subcommands {
                walk(sub, known, errs);
            }
        }

        let mut errs = Vec::new();
        for c in &doc.commands {
            walk(c, &known, &mut errs);
        }
        assert!(errs.is_empty(), "dangling forge schema refs: {errs:?}");
    }

    /// Every adopted command must pin its exact `command_id`, output mode,
    /// and schema refs. A drift in any of those is an agent-contract break.
    #[test]
    fn registered_commands_pin_stable_ids() {
        let cmd = <Forge as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &REGISTRY);
        fn find<'a>(
            c: &'a foundry_cli::introspect::CommandInfo,
            id: &str,
        ) -> Option<&'a foundry_cli::introspect::CommandInfo> {
            if c.command_id == id {
                return Some(c);
            }
            for sub in &c.subcommands {
                if let Some(found) = find(sub, id) {
                    return Some(found);
                }
            }
            None
        }
        let lookup = |id: &str| -> &foundry_cli::introspect::CommandInfo {
            doc.commands
                .iter()
                .find_map(|c| find(c, id))
                .unwrap_or_else(|| panic!("{id} missing from forge introspect"))
        };

        // (command_id, expected output_mode, expected result_schema_ref,
        // expected event_schema_ref). `session_schema_ref` must be absent
        // for every adopted command in this PR.
        let pins: &[(&str, OutputMode, &str, Option<&str>)] = &[
            ("forge.build", OutputMode::Envelope, "foundry:forge.build@v1", None),
            (
                "forge.test",
                OutputMode::Stream,
                "foundry:forge.test@v1",
                Some("foundry:forge.test.event@v1"),
            ),
            (
                "forge.script",
                OutputMode::Stream,
                "foundry:forge.script@v1",
                Some("foundry:forge.script.event@v1"),
            ),
        ];
        for (id, mode, result_ref, event_ref) in pins {
            let info = lookup(id);
            assert_eq!(info.capabilities.output_mode, *mode, "{id} output_mode drift");
            assert_eq!(
                info.capabilities.result_schema_ref.as_deref(),
                Some(*result_ref),
                "{id} result_schema_ref drift"
            );
            assert_eq!(
                info.capabilities.event_schema_ref.as_deref(),
                *event_ref,
                "{id} event_schema_ref drift"
            );
            assert_eq!(
                info.capabilities.session_schema_ref, None,
                "{id} must not declare session_schema_ref"
            );
        }
    }
}
