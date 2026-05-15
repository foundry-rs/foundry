use crate::{
    cmd::{cache::CacheSubcommands, generate::GenerateSubcommands, watch},
    opts::{Forge, ForgeSubcommand},
};
use clap::CommandFactory;
use clap_complete::generate;
use eyre::Result;
use foundry_cli::utils;
use foundry_common::{sh_warn, shell};
use foundry_evm::inspectors::cheatcodes::{ForgeContext, set_execution_context};

/// Run the `forge` command line interface.
pub fn run() -> Result<()> {
    setup()?;

    foundry_cli::machine::check_machine();
    foundry_cli::opts::GlobalArgs::check_introspect::<Forge>();
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
            if cmd.is_watch() {
                global.block_on(watch::watch_test(cmd))
            } else {
                let silent = cmd.junit || shell::is_json();
                let outcome = global.block_on(cmd.run())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use foundry_cli::introspect::{
        CommandRegistry, build_document, capability_violations, duplicate_command_ids,
    };

    /// Every `command_id` exposed by `forge --introspect` MUST be unique.
    /// This is the foundation of the agent contract — agents key on
    /// `command_id` to identify commands, and duplicates would silently break
    /// downstream tooling.
    #[test]
    fn introspect_command_ids_are_unique() {
        let cmd = <Forge as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let dups = duplicate_command_ids(&doc);
        assert!(dups.is_empty(), "duplicate forge command_ids: {dups:?}");
    }

    /// Capability self-consistency: any command declaring an output mode
    /// must wire the matching schema reference. See
    /// [`capability_violations`].
    #[test]
    fn introspect_capabilities_are_consistent() {
        let cmd = <Forge as clap::CommandFactory>::command();
        let doc = build_document(&cmd, &CommandRegistry::EMPTY);
        let v = capability_violations(&doc);
        assert!(v.is_empty(), "forge capability violations: {v:?}");
    }
}
