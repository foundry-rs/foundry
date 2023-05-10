//! Watch mode support

use crate::{
    cmd::forge::{build::BuildArgs, snapshot::SnapshotArgs, test::TestArgs},
    utils::{self, FoundryPathExt},
};
use clap::Parser;
use foundry_config::Config;
use std::{collections::HashSet, convert::Infallible, path::PathBuf, sync::Arc};
use tracing::trace;
use watchexec::{
    action::{Action, Outcome, PreSpawn},
    command::Command,
    config::{InitConfig, RuntimeConfig},
    event::{Event, Priority, ProcessEnd},
    handler::SyncFnHandler,
    paths::summarise_events_to_env,
    signal::source::MainSignal,
    Watchexec,
};

#[derive(Debug, Clone, Parser, Default)]
#[clap(next_help_heading = "Watch options")]
pub struct WatchArgs {
    /// Watch the given files or directories for changes.
    ///
    /// If no paths are provided, the source and test directories of the project are watched.
    #[clap(
        long,
        short,
        num_args(0..),
        value_name = "PATH",
    )]
    pub watch: Option<Vec<PathBuf>>,

    /// Do not restart the command while it's still running.
    #[clap(long)]
    pub no_restart: bool,

    /// Explicitly re-run all tests when a change is made.
    ///
    /// By default, only the tests of the last modified test file are executed.
    #[clap(long)]
    pub run_all: bool,

    /// File update debounce delay.
    ///
    /// During the delay, incoming change events are accumulated and
    /// only once the delay has passed, is an action taken. Note that
    /// this does not mean a command will be started: if --no-restart is
    /// given and a command is already running, the outcome of the
    /// action will be to do nothing.
    ///
    /// Defaults to 50ms. Parses as decimal seconds by default, but
    /// using an integer with the `ms` suffix may be more convenient.
    ///
    /// When using --poll mode, you'll want a larger duration, or risk
    /// overloading disk I/O.
    #[clap(long, value_name = "DELAY")]
    pub watch_delay: Option<String>,
}

impl WatchArgs {
    /// Returns new [InitConfig] and [RuntimeConfig] based on the [WatchArgs]
    ///
    /// If paths were provided as arguments the these will be used as the watcher's pathset,
    /// otherwise the path the closure returns will be used
    pub fn watchexec_config(
        &self,
        f: impl FnOnce() -> Vec<PathBuf>,
    ) -> eyre::Result<(InitConfig, RuntimeConfig)> {
        let init = init()?;
        let mut runtime = runtime(self)?;

        // contains all the arguments `--watch p1, p2, p3`
        let has_paths = self.watch.as_ref().map(|paths| !paths.is_empty()).unwrap_or_default();

        if !has_paths {
            // use alternative pathset, but only those that exists
            runtime.pathset(f().into_iter().filter(|p| p.exists()));
        }
        Ok((init, runtime))
    }
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// build`
pub async fn watch_build(args: BuildArgs) -> eyre::Result<()> {
    let (init, mut runtime) = args.watchexec_config()?;
    let cmd = cmd_args(args.watch.watch.as_ref().map(|paths| paths.len()).unwrap_or_default());

    trace!("watch build cmd={:?}", cmd);
    runtime.command(watch_command(cmd.clone()));

    let wx = Watchexec::new(init, runtime.clone())?;
    on_action(args.watch, runtime, Arc::clone(&wx), cmd, (), |_| {});

    // start executing the command immediately
    wx.send_event(Event::default(), Priority::default()).await?;
    wx.main().await??;

    Ok(())
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// snapshot`
pub async fn watch_snapshot(args: SnapshotArgs) -> eyre::Result<()> {
    let (init, mut runtime) = args.watchexec_config()?;
    let cmd = cmd_args(args.test.watch.watch.as_ref().map(|paths| paths.len()).unwrap_or_default());

    trace!("watch snapshot cmd={:?}", cmd);
    runtime.command(watch_command(cmd.clone()));
    let wx = Watchexec::new(init, runtime.clone())?;

    on_action(args.test.watch.clone(), runtime, Arc::clone(&wx), cmd, (), |_| {});

    // start executing the command immediately
    wx.send_event(Event::default(), Priority::default()).await?;
    wx.main().await??;

    Ok(())
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// test`
pub async fn watch_test(args: TestArgs) -> eyre::Result<()> {
    let (init, mut runtime) = args.watchexec_config()?;
    let cmd = cmd_args(args.watch.watch.as_ref().map(|paths| paths.len()).unwrap_or_default());
    trace!("watch test cmd={:?}", cmd);
    runtime.command(watch_command(cmd.clone()));
    let wx = Watchexec::new(init, runtime.clone())?;

    let config: Config = args.build_args().into();

    let filter = args.filter(&config);

    // marker to check whether to override the command
    let no_reconfigure = filter.args().pattern.is_some() ||
        filter.args().test_pattern.is_some() ||
        filter.args().path_pattern.is_some() ||
        filter.args().contract_pattern.is_some() ||
        args.watch.run_all;

    let state = WatchTestState {
        project_root: config.__root.0,
        no_reconfigure,
        last_test_files: Default::default(),
    };
    on_action(args.watch.clone(), runtime, Arc::clone(&wx), cmd, state, on_test);

    // start executing the command immediately
    wx.send_event(Event::default(), Priority::default()).await?;
    wx.main().await??;

    Ok(())
}

#[derive(Debug, Clone)]
struct WatchTestState {
    /// the root directory of the project
    project_root: PathBuf,
    /// marks whether we can reconfigure the watcher command with the `--match-path` arg
    no_reconfigure: bool,
    /// Tracks the last changed test files, if any so that if a non-test file was modified we run
    /// this file instead *Note:* this is a vec, so we can also watch out for changes
    /// introduced by `forge fmt`
    last_test_files: HashSet<String>,
}

/// The `on_action` hook for `forge test --watch`
fn on_test(action: OnActionState<WatchTestState>) {
    let OnActionState { args, runtime, action, wx, cmd, other } = action;
    let WatchTestState { project_root, no_reconfigure, last_test_files } = other;

    if no_reconfigure {
        // nothing to reconfigure
        return
    }

    let mut cmd = cmd.clone();

    let mut changed_sol_test_files: HashSet<_> = action
        .events
        .iter()
        .flat_map(|e| e.paths())
        .filter(|(path, _)| path.is_sol_test())
        .filter_map(|(path, _)| path.to_str())
        .map(str::to_string)
        .collect();

    // replace `--match-path` | `-mp` argument
    if let Some(pos) = cmd.iter().position(|arg| arg == "--match-path" || arg == "-mp") {
        // --match-path requires 1 argument
        cmd.drain(pos..=(pos + 1));
    }

    if changed_sol_test_files.len() > 1 ||
        (changed_sol_test_files.is_empty() && last_test_files.is_empty())
    {
        // this could happen if multiple files were changed at once, for example `forge fmt` was
        // run, or if no test files were changed and no previous test files were modified in which
        // case we simply run all
        let mut config = runtime.clone();
        config.command(watch_command(cmd.clone()));
        // re-register the action
        on_action(
            args.clone(),
            config,
            wx,
            cmd,
            WatchTestState {
                project_root,
                no_reconfigure,
                last_test_files: changed_sol_test_files,
            },
            on_test,
        );
        return
    }

    if changed_sol_test_files.is_empty() {
        // reuse the old test files if a non-test file was changed
        changed_sol_test_files = last_test_files;
    }

    // append `--match-path` glob
    let mut file = changed_sol_test_files.clone().into_iter().next().expect("test file present");

    // remove the project root dir from the detected file
    if let Some(root) = project_root.as_os_str().to_str() {
        if let Some(f) = file.strip_prefix(root) {
            file = f.trim_start_matches('/').to_string();
        }
    }

    let mut new_cmd = cmd.clone();
    new_cmd.push("--match-path".to_string());
    new_cmd.push(file);
    trace!("reconfigure test command {:?}", new_cmd);

    // reconfigure the executor with a new runtime
    let mut config = runtime.clone();
    config.command(watch_command(new_cmd));

    // re-register the action
    on_action(
        args.clone(),
        config,
        wx,
        cmd,
        WatchTestState { project_root, no_reconfigure, last_test_files: changed_sol_test_files },
        on_test,
    );
}

/// Converts a list of arguments to a `watchexec::Command`
///
/// The first index in `args`, is expected to be the path to the executable, See `cmd_args`
///
/// # Panics
/// if `args` is empty
fn watch_command(mut args: Vec<String>) -> Command {
    debug_assert!(!args.is_empty());
    let prog = args.remove(0);
    Command::Exec { prog, args }
}

/// Returns the env args without the `--watch` flag from the args for the Watchexec command
fn cmd_args(num: usize) -> Vec<String> {
    // all the forge arguments including path to forge bin
    let mut cmd_args: Vec<_> = std::env::args().collect();
    if let Some(pos) = cmd_args.iter().position(|arg| arg == "--watch" || arg == "-w") {
        cmd_args.drain(pos..=(pos + num));
    }

    cmd_args
}

/// Returns the Initialisation configuration for [`Watchexec`].
pub fn init() -> eyre::Result<InitConfig> {
    let mut config = InitConfig::default();
    config.on_error(SyncFnHandler::from(|data| -> std::result::Result<(), Infallible> {
        trace!("[[{:?}]]", data);
        Ok(())
    }));

    Ok(config)
}

/// Contains all necessary context to reconfigure a [`Watchexec`] on the fly
struct OnActionState<'a, T: Clone> {
    args: &'a WatchArgs,
    runtime: &'a RuntimeConfig,
    action: &'a Action,
    cmd: &'a Vec<String>,
    wx: Arc<Watchexec>,
    // additional context to inject
    other: T,
}

/// Registers the `on_action` hook on the `RuntimeConfig` currently in use in the `Watchexec`
///
/// **Note** this is a bit weird since we're installing the hook on the config that's already used
/// in `Watchexec` but necessary if we want to have access to it in order to
/// [`Watchexec::reconfigure`]
fn on_action<F, T>(
    args: WatchArgs,
    mut config: RuntimeConfig,
    wx: Arc<Watchexec>,
    cmd: Vec<String>,
    other: T,
    f: F,
) where
    F: for<'a> Fn(OnActionState<'a, T>) + Send + 'static,
    T: Clone + Send + 'static,
{
    let on_busy = if args.no_restart { "do-nothing" } else { "restart" };
    let runtime = config.clone();
    let w = Arc::clone(&wx);
    config.on_action(move |action: Action| {
        let fut = async { Ok::<(), Infallible>(()) };
        let signals: Vec<MainSignal> = action.events.iter().flat_map(|e| e.signals()).collect();
        let has_paths = action.events.iter().flat_map(|e| e.paths()).next().is_some();

        if signals.contains(&MainSignal::Terminate) || signals.contains(&MainSignal::Interrupt) {
            action.outcome(Outcome::both(Outcome::Stop, Outcome::Exit));
            return fut
        }

        if !has_paths {
            if !signals.is_empty() {
                let mut out = Outcome::DoNothing;
                for sig in signals {
                    out = Outcome::both(out, Outcome::Signal(sig));
                }

                action.outcome(out);
                return fut
            }

            let completion = action.events.iter().flat_map(|e| e.completions()).next();
            if let Some(status) = completion {
                match status {
                    Some(ProcessEnd::ExitError(code)) => {
                        tracing::trace!("Command exited with {code}")
                    }
                    Some(ProcessEnd::ExitSignal(sig)) => {
                        tracing::trace!("Command killed by {:?}", sig)
                    }
                    Some(ProcessEnd::ExitStop(sig)) => {
                        tracing::trace!("Command stopped by {:?}", sig)
                    }
                    Some(ProcessEnd::Continued) => tracing::trace!("Command continued"),
                    Some(ProcessEnd::Exception(ex)) => {
                        tracing::trace!("Command ended by exception {:#x}", ex)
                    }
                    Some(ProcessEnd::Success) => tracing::trace!("Command was successful"),
                    None => tracing::trace!("Command completed"),
                };

                action.outcome(Outcome::DoNothing);
                return fut
            }
        }

        f(OnActionState {
            args: &args,
            runtime: &runtime,
            action: &action,
            wx: w.clone(),
            cmd: &cmd,
            other: other.clone(),
        });

        // mattsse: could be made into flag to never clear the shell
        let clear = false;
        let when_running = match (clear, on_busy) {
            (_, "do-nothing") => Outcome::DoNothing,
            (true, "restart") => {
                Outcome::both(Outcome::Stop, Outcome::both(Outcome::Clear, Outcome::Start))
            }
            (false, "restart") => Outcome::both(Outcome::Stop, Outcome::Start),
            _ => Outcome::DoNothing,
        };

        let when_idle =
            if clear { Outcome::both(Outcome::Clear, Outcome::Start) } else { Outcome::Start };

        action.outcome(Outcome::if_running(when_running, when_idle));

        fut
    });

    let _ = wx.reconfigure(config);
}

/// Returns the Runtime configuration for [`Watchexec`].
pub fn runtime(args: &WatchArgs) -> eyre::Result<RuntimeConfig> {
    let mut config = RuntimeConfig::default();

    config.pathset(args.watch.clone().unwrap_or_default());

    if let Some(delay) = &args.watch_delay {
        config.action_throttle(utils::parse_delay(delay)?);
    }

    config.on_pre_spawn(move |prespawn: PreSpawn| async move {
        let envs = summarise_events_to_env(prespawn.events.iter());
        if let Some(mut command) = prespawn.command().await {
            for (k, v) in envs {
                command.env(format!("CARGO_WATCH_{k}_PATH"), v);
            }
        }

        Ok::<(), Infallible>(())
    });

    Ok(config)
}
