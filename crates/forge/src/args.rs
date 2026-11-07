use crate::{
    cmd::{cache::CacheSubcommands, watch},
    opts::{Forge, ForgeSubcommand},
};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use eyre::Result;
use foundry_cli::utils;
use foundry_common::{sh_warn, shell};
use foundry_evm::inspectors::cheatcodes::{ForgeContext, set_execution_context};

/// Run the `forge` command line interface.
pub fn run() -> Result<()> {
    foundry_cli::opts::GlobalArgs::check_markdown_help::<Forge>();

    let warnings = setup()?;

    let args = Forge::parse();
    args.global.init()?;
    for warning in warnings {
        let _ = foundry_common::sh_warn!("{warning}");
    }

    run_command(args)
}

/// Setup the global logger and other utilities.
pub fn setup() -> Result<Vec<String>> {
    let warnings = utils::common_setup();
    utils::subscriber();

    Ok(warnings)
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
                block_on_watch(global, watch::watch_test(cmd))
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
        ForgeSubcommand::Properties(cmd) => cmd.run(),
        ForgeSubcommand::Script(cmd) => block_on_command(global, || cmd.run_script()),
        ForgeSubcommand::Coverage(cmd) => {
            if cmd.is_watch() {
                block_on_watch(global, watch::watch_coverage(cmd))
            } else {
                global.block_on(cmd.run())
            }
        }
        ForgeSubcommand::Bind(cmd) => cmd.run(),
        ForgeSubcommand::Build { args, locked } => {
            if args.is_watch() {
                block_on_watch(global, watch::watch_build(args))
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
        ForgeSubcommand::Reinit(cmd) => cmd.run(),
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
                block_on_watch(global, watch::watch_gas_snapshot(cmd))
            } else {
                global.block_on(cmd.run())
            }
        }
        ForgeSubcommand::Fmt(cmd) => {
            if cmd.is_watch() {
                block_on_watch(global, watch::watch_fmt(cmd))
            } else {
                cmd.run()
            }
        }
        ForgeSubcommand::Config(cmd) => cmd.run(),
        ForgeSubcommand::Flatten(cmd) => cmd.run(),
        ForgeSubcommand::Inspect(cmd) => cmd.run(),
        ForgeSubcommand::Tree(cmd) => cmd.run(),
        ForgeSubcommand::Geiger(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Exploit(cmd) => global.block_on(cmd.run()),
        ForgeSubcommand::Doc(cmd) => {
            if cmd.is_watch() {
                block_on_watch(global, watch::watch_doc(cmd))
            } else {
                global.block_on(cmd.run())
            }
        }
        ForgeSubcommand::Selectors { command } => global.block_on(command.run()),
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

/// Run watch commands without waiting indefinitely for detached stdin readers to stop.
fn block_on_watch<F: std::future::Future>(
    global: &foundry_cli::opts::GlobalArgs,
    future: F,
) -> F::Output {
    let runtime = global.tokio_runtime();
    let result = runtime.block_on(future);
    // Watchexec has already stopped its child processes before returning. Its blocking stdin
    // reader can outlive the watcher, so dropping the runtime would otherwise wait indefinitely.
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc::{self, TryRecvError},
        time::Duration,
    };

    #[test]
    fn watch_shutdown_does_not_wait_for_blocking_reader() {
        let (release_tx, release_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();

        let result = block_on_watch(&Default::default(), async move {
            tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                // Bound the simulated read even if runtime shutdown regresses.
                let _ = release_rx.recv_timeout(Duration::from_secs(10));
                finished_tx.send(()).unwrap();
            });
            started_rx.await.unwrap();
            Err::<(), _>("watch failed")
        });

        assert_eq!(result, Err("watch failed"));
        assert_eq!(finished_rx.try_recv(), Err(TryRecvError::Empty));
        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    }
}
