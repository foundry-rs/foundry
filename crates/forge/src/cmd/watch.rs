use super::{
    build::BuildArgs, coverage::CoverageArgs, doc::DocArgs, fmt::FmtArgs,
    snapshot::GasSnapshotArgs, test::TestArgs,
};
use alloy_primitives::map::HashSet;
use clap::{CommandFactory, Parser};
use eyre::Result;
use foundry_cli::utils::{self, FoundryPathExt, LoadConfig};
use foundry_config::Config;
use parking_lot::Mutex;
use std::{
    ffi::OsString,
    io::IsTerminal,
    path::PathBuf,
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};
use tokio::process::Command as TokioCommand;
use watchexec::{
    Watchexec,
    action::ActionHandler,
    command::{Command, Program},
    job::{CommandState, Job},
    paths::summarise_events_to_env,
};
use watchexec_events::{
    Event, KeyCode, Keyboard, Priority, ProcessEnd, Tag,
    filekind::{AccessKind, FileEventKind},
};
use watchexec_signals::Signal;
use yansi::{Color, Paint};

type SpawnHook = Arc<dyn Fn(&[Event], &mut TokioCommand) + Send + Sync + 'static>;
type KeyboardConfig = Arc<OnceLock<Weak<watchexec::Config>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyboardAction {
    Rerun,
    Quit,
}

fn keyboard_action(events: &[Event]) -> Option<KeyboardAction> {
    let mut rerun = false;

    for tag in events.iter().flat_map(|event| &event.tags) {
        match tag {
            Tag::Keyboard(Keyboard::Eof) => return Some(KeyboardAction::Quit),
            Tag::Keyboard(Keyboard::Key { key: KeyCode::Char('a'), modifiers })
                if modifiers.is_empty() =>
            {
                rerun = true;
            }
            _ => {}
        }
    }

    rerun.then_some(KeyboardAction::Rerun)
}

fn set_keyboard_events(config: &Option<KeyboardConfig>, enable: bool) {
    if let Some(config) = config.as_ref().and_then(|config| config.get()).and_then(Weak::upgrade) {
        config.keyboard_events(enable);
    }
}

#[derive(Clone, Debug, Default, Parser)]
#[command(next_help_heading = "Watch options")]
pub struct WatchArgs {
    /// Watch the given files or directories for changes.
    ///
    /// If no paths are provided, the source and test directories of the project are watched.
    #[arg(long, short, num_args(0..), value_name = "PATH")]
    pub watch: Option<Vec<PathBuf>>,

    /// Do not restart the command while it's still running.
    #[arg(long)]
    pub no_restart: bool,

    /// Explicitly re-run all tests when a change is made.
    ///
    /// By default, only the tests of the last modified test file are executed.
    #[arg(long)]
    pub run_all: bool,

    /// Re-run only previously failed tests first when a change is made.
    ///
    /// If all previously failed tests pass, the full test suite will be run automatically.
    /// This is particularly useful for TDD workflows where you want fast feedback on failures.
    #[arg(long, alias = "rerun-failures")]
    pub rerun_failed: bool,

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
    #[arg(long, value_name = "DELAY")]
    pub watch_delay: Option<String>,
}

impl WatchArgs {
    /// Creates a new [`watchexec::Config`].
    ///
    /// If paths were provided as arguments the these will be used as the watcher's pathset,
    /// otherwise the path the closure returns will be used.
    pub fn watchexec_config<PS: IntoIterator<Item = P>, P: Into<PathBuf>>(
        &self,
        default_paths: impl FnOnce() -> Result<PS>,
    ) -> Result<watchexec::Config> {
        self.watchexec_config_generic(default_paths, None, None)
    }

    /// Creates a new [`watchexec::Config`] with a custom command spawn hook and optional keyboard
    /// events while the command is idle.
    ///
    /// If paths were provided as arguments the these will be used as the watcher's pathset,
    /// otherwise the path the closure returns will be used.
    fn watchexec_config_with_override<PS: IntoIterator<Item = P>, P: Into<PathBuf>>(
        &self,
        default_paths: impl FnOnce() -> Result<PS>,
        watch_keyboard: bool,
        spawn_hook: impl Fn(&[Event], &mut TokioCommand) + Send + Sync + 'static,
    ) -> Result<(watchexec::Config, Option<KeyboardConfig>)> {
        let keyboard_config = watch_keyboard.then(|| Arc::new(OnceLock::new()));
        let config = self.watchexec_config_generic(
            default_paths,
            Some(Arc::new(spawn_hook)),
            keyboard_config.clone(),
        )?;
        Ok((config, keyboard_config))
    }

    fn watchexec_config_generic<PS: IntoIterator<Item = P>, P: Into<PathBuf>>(
        &self,
        default_paths: impl FnOnce() -> Result<PS>,
        spawn_hook: Option<SpawnHook>,
        keyboard_config: Option<KeyboardConfig>,
    ) -> Result<watchexec::Config> {
        let mut paths = self.watch.as_deref().unwrap_or_default();
        let storage: Vec<_>;
        if paths.is_empty() {
            storage = default_paths()?.into_iter().map(Into::into).filter(|p| p.exists()).collect();
            paths = &storage;
        }
        self.watchexec_config_inner(paths, spawn_hook, keyboard_config)
    }

    fn watchexec_config_inner(
        &self,
        paths: &[PathBuf],
        spawn_hook: Option<SpawnHook>,
        keyboard_config: Option<KeyboardConfig>,
    ) -> Result<watchexec::Config> {
        let config = watchexec::Config::default();

        config.on_error(|err| {
            let _ = sh_eprintln!("[[{err:?}]]");
        });

        if let Some(delay) = &self.watch_delay {
            config.throttle(utils::parse_delay(delay)?);
        }

        config.pathset(paths.iter().map(|p| p.as_path()));

        let n_path_args = self.watch.as_deref().unwrap_or_default().len();
        let base_command = Arc::new(watch_command(cmd_args(n_path_args)));

        let id = watchexec::Id::default();
        let quit_again = Arc::new(AtomicU8::new(0));
        let stop_timeout = Duration::from_secs(5);
        let no_restart = self.no_restart;
        let stop_signal = Signal::Terminate;
        config.on_action(move |mut action| {
            let base_command = base_command.clone();
            let job = action.get_or_create_job(id, move || base_command.clone());

            let events = action.events.clone();
            let spawn_hook = spawn_hook.clone();
            job.set_spawn_hook(move |command, _| {
                // https://github.com/watchexec/watchexec/blob/72f069a8477c679e45f845219276b0bfe22fed79/crates/cli/src/emits.rs#L9
                let env = summarise_events_to_env(events.iter());
                for (k, v) in env {
                    command.command_mut().env(format!("WATCHEXEC_{k}_PATH"), v);
                }

                if let Some(spawn_hook) = &spawn_hook {
                    spawn_hook(&events, command.command_mut());
                }
            });

            let clear_screen = || {
                let _ = clearscreen::clear();
            };

            let quit = |mut action: ActionHandler| {
                match quit_again.fetch_add(1, Ordering::Relaxed) {
                    0 => {
                        let _ = sh_eprintln!(
                            "[Waiting {stop_timeout:?} for processes to exit before stopping... \
                             Ctrl-C again to exit faster]"
                        );
                        action.quit_gracefully(stop_signal, stop_timeout);
                    }
                    1 => action.quit_gracefully(Signal::ForceStop, Duration::ZERO),
                    _ => action.quit(),
                }

                action
            };

            let signals = action.signals().collect::<Vec<_>>();
            let keyboard_action = keyboard_action(&action.events);

            if signals.contains(&Signal::Terminate)
                || signals.contains(&Signal::Interrupt)
                || keyboard_action == Some(KeyboardAction::Quit)
            {
                return quit(action);
            }

            // Only filesystem, keyboard rerun, or empty synthetic events below here.
            if action.paths().next().is_none()
                && keyboard_action != Some(KeyboardAction::Rerun)
                && !action.events.iter().any(|e| e.is_empty())
            {
                debug!("no filesystem, rerun, or synthetic events, skip without doing more");
                return action;
            }

            if cfg!(target_os = "linux") && keyboard_action != Some(KeyboardAction::Rerun) {
                // Reading a file now triggers `Access(Open)` events on Linux due to:
                // https://github.com/notify-rs/notify/pull/612
                // This causes an infinite rebuild loop: the build reads a file,
                // which triggers a notification, which restarts the build, and so on.
                // To prevent this, we ignore `Access(Open)` events during event processing.
                let mut has_file_events = false;
                let mut has_synthetic_events = false;
                'outer: for e in action.events.iter() {
                    if e.is_empty() {
                        has_synthetic_events = true;
                        break;
                    }
                    for tag in &e.tags {
                        if let Tag::FileEventKind(kind) = tag
                            && !matches!(kind, FileEventKind::Access(AccessKind::Open(_))) {
                                has_file_events = true;
                                break 'outer;
                            }
                    }
                }
                if !has_file_events && !has_synthetic_events {
                    debug!("no filesystem events (other than Access(Open)) or synthetic events, skip without doing more");
                    return action;
                }
            }

            // Let the child own stdin while it runs. This keeps prompts and the debugger from
            // racing Watchexec's keyboard event source for terminal input.
            set_keyboard_events(&keyboard_config, false);

            job.run({
                let job = job.clone();
                let keyboard_config = keyboard_config.clone();
                move |context| {
                    if context.current.is_running() && no_restart {
                        return;
                    }
                    job.restart_with_signal(stop_signal, stop_timeout);
                    job.run({
                        let job = job.clone();
                        move |context| {
                            clear_screen();
                            setup_process(job, &context.command, keyboard_config)
                        }
                    });
                }
            });

            action
        });

        Ok(config)
    }
}

fn setup_process(job: Job, _command: &Command, keyboard_config: Option<KeyboardConfig>) {
    tokio::spawn(async move {
        job.to_wait().await;
        job.run(move |context| end_of_process(context.current, keyboard_config));
    });
}

fn end_of_process(state: &CommandState, keyboard_config: Option<KeyboardConfig>) {
    let CommandState::Finished { status, started, finished } = state else {
        return;
    };

    let duration = *finished - *started;
    let timings = true;
    let timing = if timings { format!(", lasted {duration:?}") } else { String::new() };
    let (msg, fg) = match status {
        ProcessEnd::ExitError(code) => (format!("Command exited with {code}{timing}"), Color::Red),
        ProcessEnd::ExitSignal(sig) => {
            (format!("Command killed by {sig:?}{timing}"), Color::Magenta)
        }
        ProcessEnd::ExitStop(sig) => (format!("Command stopped by {sig:?}{timing}"), Color::Blue),
        ProcessEnd::Continued => (format!("Command continued{timing}"), Color::Cyan),
        ProcessEnd::Exception(ex) => {
            (format!("Command ended by exception {ex:#x}{timing}"), Color::Yellow)
        }
        ProcessEnd::Success => (format!("Command was successful{timing}"), Color::Green),
    };

    let quiet = false;
    set_keyboard_events(&keyboard_config, true);
    if !quiet {
        let _ = sh_eprintln!("{}", format!("[{msg}]").paint(fg.foreground()));
    }
}

/// Runs the given [`watchexec::Config`].
pub async fn run(config: watchexec::Config) -> Result<()> {
    run_inner(config, None).await
}

async fn run_inner(
    config: watchexec::Config,
    keyboard_config: Option<KeyboardConfig>,
) -> Result<()> {
    let wx = Watchexec::with_config(config)?;
    if let Some(config) = keyboard_config {
        debug_assert!(config.set(Arc::downgrade(&wx.config)).is_ok());
    }
    wx.send_event(Event::default(), Priority::Urgent).await?;
    wx.main().await??;
    Ok(())
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// build`
pub async fn watch_build(args: BuildArgs) -> Result<()> {
    let config = args.watchexec_config()?;
    run(config).await
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// snapshot`
pub async fn watch_gas_snapshot(args: GasSnapshotArgs) -> Result<()> {
    let config = args.watchexec_config()?;
    run(config).await
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// test`
pub async fn watch_test(args: TestArgs) -> Result<()> {
    let config: Config = args.build.load_config()?;
    let filter = args.filter(&config)?;
    // Marker to check whether to override the command.
    let no_reconfigure = filter.args().test_pattern.is_some()
        || filter.args().path_pattern.is_some()
        || filter.args().contract_pattern.is_some()
        || args.watch.run_all;

    let last_test_files = Mutex::new(HashSet::<String>::default());
    let project_root = config.root.to_string_lossy().into_owned();
    let test_failures_file = config.test_failures_file.clone();
    let rerun_failed = args.watch.rerun_failed;
    let watch_keyboard = std::io::stdin().is_terminal();

    let (config, keyboard_config) = args.watch.watchexec_config_with_override(
        || Ok([&config.test, &config.src]),
        watch_keyboard,
        move |events, command| {
            if keyboard_action(events) == Some(KeyboardAction::Rerun) {
                return;
            }

            // Check if we should prioritize rerunning failed tests
            let has_failures = rerun_failed && test_failures_file.exists();

            if has_failures {
                // Smart mode: rerun failed tests first
                trace!("Smart watch mode: will rerun failed tests first");
                command.arg("--rerun");
                // Don't add file-specific filters when rerunning failures
                return;
            }

            let mut changed_sol_test_files: HashSet<_> = events
                .iter()
                .flat_map(|e| e.paths())
                .filter(|(path, _)| path.is_sol_test())
                .filter_map(|(path, _)| path.to_str())
                .map(str::to_string)
                .collect();

            if changed_sol_test_files.len() > 1 {
                // Run all tests if multiple files were changed at once, for example when running
                // `forge fmt`.
                return;
            }

            if changed_sol_test_files.is_empty() {
                // Reuse the old test files if a non-test file was changed.
                let last = last_test_files.lock();
                if last.is_empty() {
                    return;
                }
                changed_sol_test_files = last.clone();
            }

            // append `--match-path` glob
            let mut file = changed_sol_test_files.iter().next().expect("test file present").clone();

            // remove the project root dir from the detected file
            if let Some(f) = file.strip_prefix(&project_root) {
                file = f.trim_start_matches('/').to_string();
            }

            trace!(?file, "reconfigure test command");

            // Before appending `--match-path`, check if it already exists
            if !no_reconfigure {
                command.arg("--match-path").arg(file);
            }
        },
    )?;

    if watch_keyboard {
        let _ = sh_eprintln!("[Press 'a' to rerun all tests]");
    }

    run_inner(config, keyboard_config).await
}

pub async fn watch_coverage(args: CoverageArgs) -> Result<()> {
    args.ensure_mode_compatible()?;

    let config = args.watch().watchexec_config(|| {
        let config = args.load_config()?;
        Ok([config.test, config.src])
    })?;
    run(config).await
}

pub async fn watch_fmt(args: FmtArgs) -> Result<()> {
    let config = args.watch.watchexec_config(|| {
        let config = args.load_config()?;
        Ok([config.src, config.test, config.script])
    })?;
    run(config).await
}

/// Executes a [`Watchexec`] that listens for changes affecting the generated
/// documentation: project sources, optionally library sources, the README that
/// becomes the homepage, the deployments directory, and the foundry config file
/// itself. Without these, the live preview goes silently stale on common edits.
pub async fn watch_doc(args: DocArgs) -> Result<()> {
    let include_libraries = args.include_libraries;
    let deployments_arg = args.deployments.clone();
    let config = args.watch.watchexec_config(|| {
        let config = args.config()?;
        let root = config.root.clone();

        let mut paths = Vec::new();

        // Solidity sources.
        paths.push(config.src.clone());

        // External libraries when explicitly opted in.
        if include_libraries {
            for lib in &config.libs {
                paths.push(lib.clone());
            }
        }

        // Homepage source: explicit override, else <root>/README.md when present.
        if let Some(hp) = &config.doc.homepage {
            let hp_path = if hp.is_absolute() { hp.clone() } else { root.join(hp) };
            if hp_path.exists() {
                paths.push(hp_path);
            }
        }
        // Mirror vocs homepage resolution: <sources>/README.md takes priority over
        // <root>/README.md.
        let src_readme = config.src.join("README.md");
        if src_readme.exists() {
            paths.push(src_readme);
        }
        let readme = root.join("README.md");
        if readme.exists() {
            paths.push(readme);
        }

        // Deployments directory: only when the user enabled `--deployments`.
        if let Some(dir_opt) = deployments_arg.as_ref() {
            let dep_dir = match dir_opt {
                Some(p) if p.is_absolute() => p.clone(),
                Some(p) => root.join(p),
                None => root.join("deployments"),
            };
            if dep_dir.exists() {
                paths.push(dep_dir);
            }
        }

        // Foundry config file (`foundry.toml`), when present.
        let toml = root.join("foundry.toml");
        if toml.exists() {
            paths.push(toml);
        }

        Ok(paths)
    })?;
    run(config).await
}

/// Converts a list of arguments to a `watchexec::Command`.
///
/// The first index in `args` is the path to the executable.
///
/// # Panics
///
/// Panics if `args` is empty.
fn watch_command(mut args: Vec<OsString>) -> Command {
    debug_assert!(!args.is_empty());
    let prog = args.remove(0);
    Command {
        program: Program::Exec {
            prog: prog.into(),
            // `watchexec::command::Program::Exec` only takes `String` arguments, so a non-UTF-8
            // argument can only be forwarded lossily. What matters here is that collecting it no
            // longer panics, see [`cmd_args`].
            args: args.into_iter().map(|arg| arg.to_string_lossy().into_owned()).collect(),
        },
        options: Default::default(),
    }
}

/// Returns the env args without the `--watch` flag from the args for the Watchexec command
fn cmd_args(num: usize) -> Vec<OsString> {
    // `args_os` and not `args`: the latter panics on any non-UTF-8 argument, which is a real
    // possibility here since `--watch` takes `PathBuf`s. See #16645 which fixed the same problem
    // on the common CLI path.
    clean_cmd_args(num, std::env::args_os().collect())
}

/// Removes every spelling of the `--watch` flag from `cmd_args`.
///
/// `num` is the total number of paths clap parsed for `--watch`. [`WatchArgs::watch`] is an
/// `Option<Vec<PathBuf>>`, so clap gives it append semantics and the flag may appear more than
/// once.
///
/// The child command is the (almost) same command line re-executed, so any leftover `--watch`
/// makes the child spawn a watcher of its own, and so on. #5872 was one instance of this, caused
/// by `-vw`; the remaining spellings clap accepts are:
///
/// - `--watch=src`, `-w=src` — value attached with `=`
/// - `-wsrc` — value attached to the short flag
/// - `--watch src --watch test` — the flag repeated
///
/// Arguments are kept as [`OsString`], so a non-UTF-8 path never panics here; see [`cmd_args`].
///
/// Anything after a `--` separator is a positional to clap and is passed through verbatim.
///
/// [`WatchArgs::watch`]: WatchArgs::watch
#[instrument(level = "debug", ret)]
fn clean_cmd_args(num: usize, cmd_args: Vec<OsString>) -> Vec<OsString> {
    // Which short flags take a value decides whether a `w` in a cluster is the watch flag or
    // part of another option's value, see [`parse_watch_flag`].
    let value_taking_shorts = value_taking_short_flags(&cmd_args);

    // How many `--watch` values are still unaccounted for. Clap reports the total across every
    // occurrence of the flag, so this budget is what keeps us from eating the arguments of
    // whichever flag happens to follow.
    let mut values_left = num;
    let mut cleaned = Vec::with_capacity(cmd_args.len());
    let mut args = cmd_args.into_iter().peekable();
    let mut after_separator = false;

    while let Some(arg) = args.next() {
        if after_separator {
            cleaned.push(arg);
            continue;
        }
        if arg == "--" {
            after_separator = true;
            cleaned.push(arg);
            continue;
        }

        // Lossy conversion is only used to recognize the flag spellings, which are always ASCII.
        // Non-UTF-8 bytes can only ever occur in a *value*, and values are either dropped
        // together with their flag or pushed through verbatim as an `OsString`.
        let flag = parse_watch_flag(&arg.to_string_lossy(), &value_taking_shorts);

        match flag {
            None => cleaned.push(arg),
            Some(WatchFlag { keep, values }) => {
                if let Some(keep) = keep {
                    cleaned.push(OsString::from(keep));
                }
                match values {
                    WatchValues::Inline => values_left = values_left.saturating_sub(1),
                    WatchValues::Trailing => {
                        // Only the tokens clap actually assigned to the flag, and only until the
                        // next flag or the `--` separator starts.
                        while values_left > 0 && args.peek().is_some_and(|next| !is_flag(next)) {
                            args.next();
                            values_left -= 1;
                        }
                    }
                }
            }
        }
    }

    cleaned
}

/// The short flags that take a value in the `forge` subcommand being run.
///
/// This is the set [`parse_watch_flag`] needs to know when it may read a `w` inside a short flag
/// cluster as `--watch`.
fn value_taking_short_flags(argv: &[OsString]) -> HashSet<char> {
    fn collect_shorts(cmd: &clap::Command, shorts: &mut HashSet<char>) {
        shorts.extend(
            cmd.get_arguments()
                .filter(|arg| arg.get_action().takes_values())
                .filter_map(|arg| arg.get_short()),
        );
    }

    let cmd = crate::opts::Forge::command();
    let mut shorts = HashSet::default();
    collect_shorts(&cmd, &mut shorts);

    // Descend into the subcommand that is actually running, so that a short flag which only takes
    // a value in some other subcommand does not stop the scan.
    let mut current = &cmd;
    let mut descended = false;
    for arg in argv.iter().skip(1) {
        let Some(name) = arg.to_str() else { break };
        if name == "--" {
            break;
        }
        match current
            .get_subcommands()
            .find(|cmd| cmd.get_name() == name || cmd.get_all_aliases().any(|a| a == name))
        {
            Some(subcommand) => {
                collect_shorts(subcommand, &mut shorts);
                current = subcommand;
                descended = true;
            }
            // Global options are allowed before the subcommand; nothing else is.
            None if !descended && name.starts_with('-') => {}
            None => break,
        }
    }

    shorts
}

/// Whether an argv token starts a new flag rather than being a value of the preceding one.
///
/// A lone `-` is a value to clap rather than an option, so it may well be a watch path.
fn is_flag(arg: &OsString) -> bool {
    let arg = arg.to_string_lossy();
    arg.len() > 1 && arg.starts_with('-')
}

/// How the values of a `--watch` occurrence are spelled out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchValues {
    /// The value is part of the flag token itself: `--watch=src`, `-wsrc`, `-w=src`.
    Inline,
    /// The values are the tokens following the flag: `--watch src test`, `-vw src`.
    Trailing,
}

/// How a single argv token spells out the `--watch` flag.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WatchFlag {
    /// The short flags that precede the `w` when it is concatenated with other short flags:
    /// `-vw` keeps `-v`. `None` means the whole token belongs to `--watch` and is dropped.
    keep: Option<String>,
    /// How the values of this occurrence are spelled out.
    values: WatchValues,
}

/// Parses a single argv token as a spelling of the `--watch` flag, or returns `None` if the token
/// is unrelated to `--watch`.
///
/// `value_taking_shorts` holds the short flags that take a value in the running subcommand, see
/// [`value_taking_short_flags`].
fn parse_watch_flag(arg: &str, value_taking_shorts: &HashSet<char>) -> Option<WatchFlag> {
    if let Some(rest) = arg.strip_prefix("--watch") {
        // `--watch` or `--watch=<value>`; anything else (`--watch-delay`) is a different flag.
        return match rest.strip_prefix('=') {
            Some(_) => Some(WatchFlag { keep: None, values: WatchValues::Inline }),
            None => {
                rest.is_empty().then_some(WatchFlag { keep: None, values: WatchValues::Trailing })
            }
        };
    }

    // Short flag cluster: `-w`, `-w=src`, `-wsrc`, `-vw`, `-vwsrc`, ...
    if arg.starts_with("--") || !arg.starts_with('-') {
        return None;
    }
    // The first option of a cluster that takes a value swallows the rest of it, so a `w` behind
    // one belongs to that option rather than to `--watch`: in `-Dwarnings` it is part of the
    // value of `-D`, and in `-ooutwatch` part of the output directory.
    let mut w = None;
    for (index, short) in arg.char_indices().skip(1) {
        if short == 'w' {
            w = Some(index);
            break;
        }
        if value_taking_shorts.contains(&short) {
            return None;
        }
    }
    let w = w?;
    // `w` is ASCII, so `w + 1` is the byte right after it.
    let after = &arg[w + 1..];
    let keep = (w > 1).then(|| arg[..w].to_string());
    let values = if after.is_empty() {
        WatchValues::Trailing
    } else {
        // Either the value itself (`-wsrc`) or an `=`-attached one (`-w=src`).
        WatchValues::Inline
    };
    Some(WatchFlag { keep, values })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use watchexec_events::Modifiers;

    /// Builds an argv from `&str` literals.
    fn argv<const N: usize>(args: [&str; N]) -> Vec<OsString> {
        args.into_iter().map(OsString::from).collect()
    }

    fn key_event(key: char, modifiers: Modifiers) -> Event {
        Event {
            tags: vec![Tag::Keyboard(Keyboard::Key { key: KeyCode::Char(key), modifiers })],
            ..Default::default()
        }
    }

    #[test]
    fn classifies_keyboard_actions() {
        assert_eq!(
            keyboard_action(&[key_event('a', Modifiers::default())]),
            Some(KeyboardAction::Rerun)
        );
        assert_eq!(keyboard_action(&[key_event('x', Modifiers::default())]), None);
        assert_eq!(
            keyboard_action(&[key_event('a', Modifiers { ctrl: true, ..Default::default() })]),
            None
        );

        let eof = Event { tags: vec![Tag::Keyboard(Keyboard::Eof)], ..Default::default() };
        assert_eq!(
            keyboard_action(&[key_event('a', Modifiers::default()), eof]),
            Some(KeyboardAction::Quit)
        );
    }

    #[test]
    fn parse_cmd_args() {
        let cleaned = clean_cmd_args(0, argv(["-vw"]));
        assert_eq!(cleaned, argv(["-v"]));
    }

    #[test]
    fn locked_survives_cleaning_watch_args() {
        let args = argv(["forge", "build", "--locked", "--watch", "src", "test"]);
        assert_eq!(clean_cmd_args(2, args), argv(["forge", "build", "--locked"]));

        let args = argv(["forge", "build", "--watch", "src", "--locked"]);
        assert_eq!(clean_cmd_args(1, args), argv(["forge", "build", "--locked"]));
    }

    /// Clap accepts values attached to the flag with `=` or directly to the short flag, none of
    /// which the previous `arg == "--watch"` check matched. `-wsrc` was even rewritten into
    /// `-src`, i.e. into a completely different flag.
    #[test]
    fn cleans_inline_watch_values() {
        let cleaned = clean_cmd_args(1, argv(["forge", "build", "--watch=src"]));
        assert_eq!(cleaned, argv(["forge", "build"]));
        let cleaned = clean_cmd_args(1, argv(["forge", "build", "-w=src"]));
        assert_eq!(cleaned, argv(["forge", "build"]));
        let cleaned = clean_cmd_args(1, argv(["forge", "build", "-wsrc"]));
        assert_eq!(cleaned, argv(["forge", "build"]));
        let cleaned = clean_cmd_args(1, argv(["forge", "build", "-vwsrc"]));
        assert_eq!(cleaned, argv(["forge", "build", "-v"]));
    }

    /// `--watch` is repeatable, and every occurrence has to go, not just the first one.
    #[test]
    fn cleans_repeated_watch_flags() {
        let args = argv(["forge", "build", "--watch", "src", "--watch", "test"]);
        assert_eq!(clean_cmd_args(2, args), argv(["forge", "build"]));

        let args = argv(["forge", "build", "--watch=src", "--watch", "test"]);
        assert_eq!(clean_cmd_args(2, args), argv(["forge", "build"]));
    }

    /// A `w` behind a short option that takes a value belongs to that option's value, not to
    /// `--watch`: `-Dwarnings` and `-ooutwatch` must survive untouched.
    #[test]
    fn stops_scanning_short_options_at_first_value_taking_option() {
        let cleaned = clean_cmd_args(0, argv(["forge", "build", "-vw", "-Dwarnings"]));
        assert_eq!(cleaned, argv(["forge", "build", "-v", "-Dwarnings"]));

        let cleaned = clean_cmd_args(0, argv(["forge", "build", "-ooutwatch", "-w"]));
        assert_eq!(cleaned, argv(["forge", "build", "-ooutwatch"]));
    }

    /// Everything after a `--` separator is a positional to clap and must be forwarded verbatim.
    #[test]
    fn preserves_args_after_separator() {
        let args = argv(["forge", "fmt", "--watch", "--", "--watch=Example.sol"]);
        assert_eq!(clean_cmd_args(0, args), argv(["forge", "fmt", "--", "--watch=Example.sol"]));
    }

    /// A lone `-` is a value to clap, so it is allowed as a watch path.
    #[test]
    fn treats_standalone_dash_as_watch_value() {
        let cleaned = clean_cmd_args(1, argv(["forge", "fmt", "--watch", "-"]));
        assert_eq!(cleaned, argv(["forge", "fmt"]));

        let cleaned = clean_cmd_args(2, argv(["forge", "fmt", "--watch", "-", "src"]));
        assert_eq!(cleaned, argv(["forge", "fmt"]));
    }

    /// Flags that merely start with `--watch` must be left alone.
    #[test]
    fn keeps_unrelated_watch_prefixed_flags() {
        let args = argv(["forge", "build", "--watch-delay", "100ms"]);
        assert_eq!(clean_cmd_args(0, args.clone()), args);
    }

    /// Only as many trailing values as clap actually assigned to `--watch` are consumed.
    #[test]
    fn does_not_consume_unrelated_args() {
        assert_eq!(
            clean_cmd_args(0, argv(["forge", "build", "--watch", "--locked"])),
            argv(["forge", "build", "--locked"])
        );
        assert_eq!(
            clean_cmd_args(1, argv(["forge", "build", "-vw", "src", "--locked"])),
            argv(["forge", "build", "-v", "--locked"])
        );
    }

    /// `--watch` takes `PathBuf`s, so a non-UTF-8 path is valid input on Unix. Collecting the
    /// argv must not panic on it, and the flag must still be recognized and removed.
    #[cfg(unix)]
    #[test]
    fn cleans_non_utf8_watch_values() {
        use std::os::unix::ffi::OsStrExt;

        let bad_path = OsStr::from_bytes(&[b's', b'r', b'c', 0xff]);

        let mut cmd_args = argv(["forge", "build", "--watch"]);
        cmd_args.push(bad_path.to_os_string());
        assert_eq!(clean_cmd_args(1, cmd_args), argv(["forge", "build"]));

        let mut inline = OsString::from("--watch=");
        inline.push(bad_path);
        let cmd_args = argv(["forge", "build"]).into_iter().chain([inline]).collect();
        assert_eq!(clean_cmd_args(1, cmd_args), argv(["forge", "build"]));
    }
}
