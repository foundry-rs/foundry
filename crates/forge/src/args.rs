use crate::{
    cmd::{cache::CacheSubcommands, generate::GenerateSubcommands, watch},
    opts::{Forge, ForgeSubcommand},
};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use eyre::Result;
use foundry_cli::utils;
use foundry_common::{sh_warn, shell};
use foundry_evm::inspectors::cheatcodes::{ForgeContext, set_execution_context};
use std::ffi::OsStr;

/// Run the `forge` command line interface.
pub fn run() -> Result<()> {
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Forge>();

    // LSP owns stdin/stdout for the lifetime of the process. Do not let the normal setup path
    // inspect dotenv files before clap has parsed the command, since an approval prompt would
    // consume an LSP message from stdin.
    if is_lsp_invocation(std::env::args_os()) {
        let args = Forge::parse();
        return run_lsp(args);
    }

    setup()?;

    let args = Forge::parse();
    args.global.init()?;

    run_command(args)
}

fn run_lsp(args: Forge) -> Result<()> {
    let Forge { global, cmd: ForgeSubcommand::Lsp(cmd) } = args else {
        unreachable!("LSP invocation must parse the LSP subcommand");
    };

    global.block_on(crate::cmd::lsp::run(cmd))
}

fn is_lsp_invocation<I>(args: I) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<OsStr>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    let mut skip_next = false;

    for arg in args {
        let arg = arg.as_ref();
        if skip_next {
            skip_next = false;
            continue;
        }

        if arg == "--" {
            return false;
        }

        if matches!(
            arg,
            s if s == "-q"
                || s == "--quiet"
                || s == "--silent"
                || s == "--json"
                || s == "--format-json"
                || s == "--md"
                || s == "--markdown"
                || s == "--allow-local-compiler"
                || s == "--allow-project-env"
        ) {
            continue;
        }

        if matches!(
            arg,
            s if s == "--color"
                || s == "--threads"
                || s == "--jobs"
                || s == "--profile"
                || s == "-j"
        ) {
            skip_next = true;
            continue;
        }

        let Some(arg) = arg.to_str() else {
            return false;
        };

        if arg.starts_with("--color=")
            || arg.starts_with("--threads=")
            || arg.starts_with("--jobs=")
            || arg.starts_with("--profile=")
            || arg.starts_with("-j")
            || arg.starts_with("-v")
            || arg == "--verbosity"
        {
            continue;
        }

        if arg.starts_with('-') {
            return false;
        }

        return arg == "lsp";
    }

    false
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<()> {
    utils::common_setup::<Forge>()?;
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
        ForgeSubcommand::Fuzz(cmd) => {
            let silent = cmd.is_junit() || shell::is_json();
            let outcome = global.block_on(cmd.run())?;
            outcome.ensure_ok(silent)
        }
        ForgeSubcommand::Script(cmd) => block_on_command(global, || cmd.run_script()),
        ForgeSubcommand::Coverage(cmd) => {
            if cmd.is_watch() {
                global.block_on(watch::watch_coverage(cmd))
            } else {
                global.block_on(cmd.run())
            }
        }
        ForgeSubcommand::Bind(cmd) => cmd.run(),
        ForgeSubcommand::Build { args, locked } => {
            if args.is_watch() {
                global.block_on(watch::watch_build(args))
            } else {
                global.block_on(args.run(locked)).map(drop)
            }
        }
        ForgeSubcommand::VerifyContract(mut args) => {
            args.print_submission_result_to_stdout = true;
            global.block_on(args.run())
        }
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
        ForgeSubcommand::Geiger(cmd) => global.block_on(cmd.run()),
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
        ForgeSubcommand::Lint(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Lsp(cmd) => global.block_on(crate::cmd::lsp::run(cmd)),
    }
}

fn block_on_command<Fut>(
    global: &foundry_cli::opts::GlobalArgs,
    make_future: impl FnOnce() -> Fut,
) -> Fut::Output
where
    Fut: std::future::Future,
{
    global.block_on(make_future())
}

#[cfg(test)]
mod tests {
    use super::is_lsp_invocation;

    #[test]
    fn detects_lsp_after_global_options() {
        assert!(is_lsp_invocation([
            "forge",
            "--quiet",
            "--threads",
            "2",
            "--profile=ci",
            "lsp",
            "--stdio",
        ]));
        assert!(is_lsp_invocation(["forge", "--jobs", "2", "lsp"]));
        assert!(is_lsp_invocation(["forge", "--jobs=2", "lsp"]));
    }

    #[test]
    fn does_not_treat_option_values_or_other_commands_as_lsp() {
        assert!(!is_lsp_invocation(["forge", "--profile", "lsp", "test"]));
        assert!(!is_lsp_invocation(["forge", "build", "lsp"]));
        assert!(!is_lsp_invocation(["forge", "--", "lsp"]));
        assert!(!is_lsp_invocation(["forge", "--version"]));
    }
}
