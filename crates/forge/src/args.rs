use crate::{
    cmd::{cache::CacheSubcommands, generate::GenerateSubcommands, watch},
    opts::{Forge, ForgeSubcommand},
};
use clap::{
    Command, CommandFactory, FromArgMatches, Parser, builder::OsStringValueParser,
    error::ErrorKind, parser::ValueSource,
};
use clap_complete::generate;
use eyre::Result;
use foundry_cli::utils;
use foundry_common::{sh_warn, shell};
use foundry_evm::inspectors::cheatcodes::{ForgeContext, set_execution_context};
use std::ffi::OsString;

/// Run the `forge` command line interface.
pub fn run() -> Result<()> {
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Forge>();

    // LSP owns stdin/stdout for the lifetime of the process. Do not let the normal setup path
    // inspect dotenv files before clap has parsed the command, since an approval prompt would
    // consume an LSP message from stdin.
    if is_lsp_invocation(std::env::args_os()) {
        let args = parse_lsp_args(std::env::args_os());
        return run_lsp(args);
    }

    setup()?;

    let args = Forge::parse();
    args.global.init()?;

    run_command(args)
}

fn run_lsp(args: Forge) -> Result<()> {
    reject_unsupported_lsp_globals(std::env::args_os())?;

    let Forge { global, cmd: ForgeSubcommand::Lsp(cmd) } = args else {
        unreachable!("LSP invocation must parse the LSP subcommand");
    };

    global.block_on(crate::cmd::lsp::run(cmd))
}

fn is_lsp_invocation<I>(args: I) -> bool
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    parse_lsp_subcommand(args)
}

fn parse_lsp_subcommand<I>(args: I) -> bool
where
    I: IntoIterator<Item = OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    let Some(matches) = Forge::command()
        .disable_help_flag(true)
        .ignore_errors(true)
        .mut_args(|arg| {
            if arg.get_action().takes_values() {
                arg.value_parser(OsStringValueParser::new())
            } else {
                arg
            }
        })
        .try_get_matches_from(args.clone())
        .ok()
    else {
        return false;
    };

    if matches.subcommand_name() == Some("lsp") {
        return true;
    }

    // A typed global value can consume `lsp` and hide a malformed subcommand from the
    // permissive parse above. Only use the strict parse for this no-subcommand case so a
    // value followed by a real command (for example `--color lsp test`) is left alone.
    matches.subcommand_name().is_none() && has_invalid_lsp_value(&args)
}

fn has_invalid_lsp_value(args: &[OsString]) -> bool {
    if !args.iter().skip(1).any(|arg| arg == "lsp") {
        return false;
    }

    let Err(err) = Forge::command().disable_help_flag(true).try_get_matches_from(args) else {
        return false;
    };
    matches!(err.kind(), ErrorKind::InvalidValue | ErrorKind::ValueValidation)
}

fn lsp_command() -> Command {
    let mut command = Forge::command();
    // Clap propagates global arguments while building the command tree, so hide them afterward.
    command.build();
    let lsp = command.find_subcommand_mut("lsp").expect("forge must define the lsp subcommand");
    *lsp =
        std::mem::take(lsp).mut_args(|arg| if arg.is_global_set() { arg.hide(true) } else { arg });
    command
}

fn parse_lsp_args<I>(args: I) -> Forge
where
    I: IntoIterator,
    I::Item: Into<OsString> + Clone,
{
    let mut matches = lsp_command().get_matches_from(args);
    Forge::from_arg_matches_mut(&mut matches).unwrap_or_else(|err| err.exit())
}

fn reject_unsupported_lsp_globals<I>(args: I) -> Result<()>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let mut command = Forge::command().disable_help_flag(true).ignore_errors(true);
    let Ok(matches) = command.try_get_matches_from_mut(args) else {
        return Ok(());
    };

    let unsupported = command
        .get_arguments()
        .filter(|arg| arg.is_global_set())
        .filter(|arg| matches.value_source(arg.get_id().as_str()) == Some(ValueSource::CommandLine))
        .map(|arg| {
            arg.get_long()
                .map(|long| format!("--{long}"))
                .or_else(|| arg.get_short().map(|short| format!("-{short}")))
                .unwrap_or_else(|| arg.get_id().as_str().to_string())
        })
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        eyre::bail!("forge lsp does not support global option(s): {}", unsupported.join(", "));
    }
    Ok(())
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
    use super::{is_lsp_invocation, lsp_command, reject_unsupported_lsp_globals};

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
        assert!(is_lsp_invocation(["forge", "--threads", "2", "lsp"]));
        assert!(is_lsp_invocation(["forge", "--jobs", "2", "lsp"]));
        assert!(is_lsp_invocation(["forge", "--jobs=2", "lsp"]));
        assert!(is_lsp_invocation(["forge", "--threads=bad", "lsp"]));
        assert!(is_lsp_invocation(["forge", "--color=bogus", "lsp"]));
        assert!(is_lsp_invocation(["forge", "lsp", "--stdio"]));
        assert!(is_lsp_invocation(["forge", "lsp", "--help"]));
        assert!(is_lsp_invocation(["forge", "lsp", "-qh"]));
    }

    #[test]
    fn detects_lsp_value_validation_before_setup() {
        for args in [
            ["forge", "--color", "lsp"],
            ["forge", "--threads", "lsp"],
            ["forge", "--jobs", "lsp"],
            ["forge", "-j", "lsp"],
        ] {
            assert!(is_lsp_invocation(args));
        }
    }

    #[test]
    fn does_not_treat_option_values_or_other_commands_as_lsp() {
        assert!(!is_lsp_invocation(["forge", "--profile", "lsp", "test"]));
        assert!(!is_lsp_invocation(["forge", "build", "lsp"]));
        assert!(!is_lsp_invocation(["forge", "--", "lsp"]));
        assert!(!is_lsp_invocation(["forge", "--help", "lsp"]));
        assert!(!is_lsp_invocation(["forge", "--version"]));
        assert!(!is_lsp_invocation(["forge", "--profile", "lsp"]));
        assert!(!is_lsp_invocation(["forge", "--color", "lsp", "test"]));
        assert!(!is_lsp_invocation(["forge", "--threads", "lsp", "test"]));
    }

    #[test]
    fn rejects_lsp_globals_that_solar_does_not_consume() {
        assert!(reject_unsupported_lsp_globals(["forge", "lsp", "--profile", "ci"]).is_err());
        assert!(reject_unsupported_lsp_globals(["forge", "lsp", "--quiet"]).is_err());
        assert!(reject_unsupported_lsp_globals(["forge", "lsp", "--threads", "2"]).is_err());
        assert!(reject_unsupported_lsp_globals(["forge", "lsp", "--jobs", "2"]).is_err());
    }

    #[test]
    fn hides_unsupported_lsp_globals_from_help() {
        let mut command = lsp_command();
        let help = command
            .find_subcommand_mut("lsp")
            .expect("forge must define the lsp subcommand")
            .render_long_help()
            .to_string();

        for option in [
            "--profile",
            "--quiet",
            "--json",
            "--md",
            "--color",
            "--verbosity",
            "--allow-local-compiler",
            "--allow-project-env",
            "--threads",
            "--jobs",
        ] {
            assert!(!help.contains(option), "unexpected {option} in LSP help");
        }
    }
}
