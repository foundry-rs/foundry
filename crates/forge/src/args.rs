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
    // Pre-parse discovery flags run before `setup()` so they cannot be blocked
    // by panic-handler / tracing init failures and avoid that init's cost.
    foundry_cli::machine::check_machine();
    foundry_cli::opts::GlobalArgs::check_introspect_with(Forge::command, &REGISTRY);
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Forge>();

    setup()?;

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
        ForgeSubcommand::Test(_) | ForgeSubcommand::Fuzz(_) => ForgeContext::Test,
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

    // Reject `--machine` for forge subcommands not declared adopted in the
    // introspect registry. Without this, embedders that wrap `TestArgs` (e.g.
    // `snapshot`, `coverage`) would emit `forge.test` stream events on the
    // process-global `is_machine()` flag without ever emitting a terminal
    // envelope — spoofing `command_id` and leaving the stream unterminated.
    if foundry_cli::is_machine() {
        let adopted = matches!(args.cmd, ForgeSubcommand::Build(_) | ForgeSubcommand::Test(_));
        if !adopted {
            let name = subcommand_name(&args.cmd);
            foundry_cli::machine::bail_machine_usage_with_details(
                format!(
                    "`forge {name}` is not yet adopted for `--machine`; only \
                     `forge build` and `forge test` are. Run without `--machine` \
                     or use an adopted subcommand."
                ),
                serde_json::json!({ "subcommand": name }),
            );
        }
    }

    // Run the subcommand.
    match args.cmd {
        ForgeSubcommand::Test(cmd) => {
            // Preflight before watcher dispatch so `--watch` is rejected too.
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
        ForgeSubcommand::Fuzz(cmd) => {
            let outcome = global.block_on(cmd.run())?;
            outcome.ensure_ok(shell::is_json())
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
            cmd.ensure_machine_compatible();
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
                global.block_on(cmd.run())
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

/// Human-readable subcommand name (e.g. `"snapshot"`) for diagnostics.
const fn subcommand_name(cmd: &ForgeSubcommand) -> &'static str {
    match cmd {
        ForgeSubcommand::Test(_) => "test",
        ForgeSubcommand::Fuzz(_) => "fuzz",
        ForgeSubcommand::Script(_) => "script",
        ForgeSubcommand::Coverage(_) => "coverage",
        ForgeSubcommand::Bind(_) => "bind",
        ForgeSubcommand::Build(_) => "build",
        ForgeSubcommand::VerifyContract(_) => "verify-contract",
        ForgeSubcommand::VerifyCheck(_) => "verify-check",
        ForgeSubcommand::VerifyBytecode(_) => "verify-bytecode",
        ForgeSubcommand::Clone(_) => "clone",
        ForgeSubcommand::Cache(_) => "cache",
        ForgeSubcommand::Create(_) => "create",
        ForgeSubcommand::Update(_) => "update",
        ForgeSubcommand::Install(_) => "install",
        ForgeSubcommand::Remove(_) => "remove",
        ForgeSubcommand::Remappings(_) => "remappings",
        ForgeSubcommand::Init(_) => "init",
        ForgeSubcommand::Completions { .. } => "completions",
        ForgeSubcommand::Clean { .. } => "clean",
        ForgeSubcommand::Snapshot(_) => "snapshot",
        ForgeSubcommand::Fmt(_) => "fmt",
        ForgeSubcommand::Config(_) => "config",
        ForgeSubcommand::Flatten(_) => "flatten",
        ForgeSubcommand::Inspect(_) => "inspect",
        ForgeSubcommand::Tree(_) => "tree",
        ForgeSubcommand::Geiger(_) => "geiger",
        ForgeSubcommand::Doc(_) => "doc",
        ForgeSubcommand::Selectors { .. } => "selectors",
        ForgeSubcommand::Generate(_) => "generate",
        ForgeSubcommand::Compiler(_) => "compiler",
        ForgeSubcommand::Soldeer(_) => "soldeer",
        ForgeSubcommand::Eip712(_) => "eip712",
        ForgeSubcommand::BindJson(_) => "bind-json",
        ForgeSubcommand::Lint(_) => "lint",
    }
}

/// Emit the terminal `forge test` envelope and exit appropriately under
/// `--machine`. Bypasses [`TestOutcome::ensure_ok`]'s human output.
fn finalize_test_machine_mode(outcome: TestOutcome, wall_clock: std::time::Duration) -> Result<()> {
    let summary = TestSummaryData::from_outcome(&outcome, wall_clock);
    let warnings = aggregate_test_warnings(&outcome);

    // `--allow-failure`: success envelope + exit 0 even if `summary.failed > 0`.
    if outcome.allow_failure || outcome.failed() == 0 {
        print_json(&JsonEnvelope::success_with_warnings(summary, warnings))?;
        return Ok(());
    }
    let details = serde_json::to_value(&summary).expect("TestSummaryData is plain scalar fields");
    let failing_suites = outcome.results.values().filter(|s| s.failed() > 0).count();
    let message = format!(
        "{} test(s) failed across {} failing suite(s) (out of {} ran)",
        outcome.failed(),
        failing_suites,
        outcome.results.len(),
    );
    let mut envelope = JsonEnvelope::error(
        JsonMessage::error(foundry_cli::diagnostic::test::FAILED, message).with_details(details),
    );
    envelope.warnings = warnings;
    print_json(&envelope)?;
    std::process::exit(foundry_cli::ExitCode::TestFailure.to_i32());
}

/// Flatten per-suite warnings into envelope messages keyed by `test.warning`.
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
        CommandRegistry, INTROSPECT_SCHEMA_ID, IntrospectDocument, OutputMode, build_document,
        capability_violations, duplicate_command_ids, render_introspect_document,
    };

    /// Every `command_id` exposed by `forge --introspect` MUST be unique.
    /// This is the foundation of the agent contract — agents key on
    /// `command_id` to identify commands, and duplicates would silently break
    /// downstream tooling.
    #[test]
    fn introspect_command_ids_are_unique() {
        let cmd = Forge::command();
        let doc = build_document(&cmd, &REGISTRY);
        let dups = duplicate_command_ids(&doc);
        assert!(dups.is_empty(), "duplicate forge command_ids: {dups:?}");
    }

    /// `forge --introspect` must produce a JSON document that parses back into
    /// the canonical `IntrospectDocument` shape.
    #[test]
    fn introspect_document_is_valid_json() {
        let cmd = Forge::command();
        let json = render_introspect_document(&cmd, &CommandRegistry::EMPTY);
        let doc: IntrospectDocument = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(doc.schema_id, INTROSPECT_SCHEMA_ID);
        assert_eq!(doc.binary.name, "forge");
    }

    /// Capability self-consistency: any command declaring an output mode
    /// must wire the matching schema reference. See
    /// [`capability_violations`].
    #[test]
    fn introspect_capabilities_are_consistent() {
        let cmd = Forge::command();
        let doc = build_document(&cmd, &REGISTRY);
        let v = capability_violations(&doc);
        assert!(v.is_empty(), "forge capability violations: {v:?}");
    }

    /// Every adopted command must pin a stable `command_id` matching its
    /// registry entry. Catches accidental drift between the registry and the
    /// clap tree across both envelope- and stream-mode commands.
    #[test]
    fn registered_commands_pin_stable_ids() {
        let cmd = Forge::command();
        let doc = build_document(&cmd, &REGISTRY);
        fn walk(c: &foundry_cli::introspect::CommandInfo) -> Vec<(&str, OutputMode)> {
            let mut out = Vec::new();
            if !matches!(c.capabilities.output_mode, OutputMode::None) {
                out.push((c.command_id.as_str(), c.capabilities.output_mode));
            }
            for sub in &c.subcommands {
                out.extend(walk(sub));
            }
            out
        }
        let pinned: Vec<(&str, OutputMode)> = doc.commands.iter().flat_map(walk).collect();
        let pinned_ids: Vec<&str> = pinned.iter().map(|(id, _)| *id).collect();
        for id in ["forge.build", "forge.test"] {
            assert!(pinned_ids.contains(&id), "{id} missing from pinned ids: {pinned_ids:?}");
        }
        assert!(
            pinned.iter().any(|(id, m)| *id == "forge.test" && matches!(m, OutputMode::Stream)),
            "forge.test must be Stream: {pinned:?}"
        );
    }
}
