use super::{
    build::BuildArgs, coverage::CoverageArgs, doc::DocArgs, fmt::FmtArgs,
    snapshot::GasSnapshotArgs, test::TestArgs,
};
use crate::opts::Forge;
use alloy_primitives::map::HashSet;
use clap::{ArgAction, Command as ClapCommand, CommandFactory, Parser};
use eyre::Result;
use foundry_cli::utils::{self, FoundryPathExt, LoadConfig};
use foundry_config::Config;
use parking_lot::Mutex;
use std::{
    ffi::{OsStr, OsString},
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

        let base_command = Arc::new(watch_command(cmd_args())?);

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
fn watch_command(mut args: Vec<OsString>) -> Result<Command> {
    debug_assert!(!args.is_empty());
    let prog = PathBuf::from(args.remove(0));
    let args = args
        .into_iter()
        .map(|arg| {
            arg.into_string().map_err(|arg| {
                eyre::eyre!(
                    "watchexec requires UTF-8 command arguments; {arg:?} is not valid UTF-8"
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Command { program: Program::Exec { prog, args }, options: Default::default() })
}

/// Returns the env args without the `--watch` flag from the args for the Watchexec command.
///
/// `args_os` is used instead of `args` because the latter panics on an argument that is not valid
/// Unicode, and a watched path is a `PathBuf`, which is allowed to be non-Unicode on Unix.
fn cmd_args() -> Vec<OsString> {
    clean_cmd_args(std::env::args_os().collect())
}

/// How a command-line argument relates to the `watch` flag.
enum WatchFlag {
    /// Not the `watch` flag.
    Other,
    /// `--watch`, `-w`, or a short flag cluster ending in `w`, with the paths given as the
    /// arguments that follow it. Holds the valueless short flags leading up to `w`, which are kept.
    Bare(Option<OsString>),
    /// `--watch=<PATH>`, `-w<PATH>`, or a short flag cluster with the path attached to its `w`.
    /// Holds the valueless short flags leading up to `w`, which are kept.
    Inline(Option<OsString>),
}

/// Removes the `--watch` flag from the args for the Watchexec command, so that the child command
/// does not enter watch mode itself and spawn another watcher.
///
/// `watch` is an `Option<Vec<PathBuf>>`, so clap accepts the flag as `--watch <PATH>`,
/// `--watch=<PATH>`, `-w <PATH>`, and `-w<PATH>`, combines it with other short flags (`-vw`,
/// `-vw<PATH>`), and allows it to repeat, appending the paths of every occurrence.
#[instrument(level = "debug", ret)]
fn clean_cmd_args(cmd_args: Vec<OsString>) -> Vec<OsString> {
    let mut cleaned = Vec::with_capacity(cmd_args.len());
    let value_less_short_flags = value_less_short_flags(&cmd_args);
    let mut args = cmd_args.into_iter().peekable();

    while let Some(arg) = args.next() {
        if arg == "--" {
            cleaned.push(arg);
            cleaned.extend(args);
            break;
        }

        match watch_flag(&arg, &value_less_short_flags) {
            WatchFlag::Other => cleaned.push(arg),
            WatchFlag::Inline(kept) => cleaned.extend(kept),
            WatchFlag::Bare(kept) => {
                cleaned.extend(kept);
                // clap consumes the values of a `num_args(0..)` argument greedily, up to the next
                // option-like argument, so the paths follow the flag until one of those.
                while args.peek().is_some_and(|next| !looks_like_option(next)) {
                    args.next();
                }
            }
        }
    }

    cleaned
}

/// Returns how `arg` relates to the `watch` flag.
fn watch_flag(arg: &OsStr, value_less_short_flags: &HashSet<char>) -> WatchFlag {
    let bytes = arg.as_encoded_bytes();
    if bytes == b"--watch" || bytes == b"-w" {
        return WatchFlag::Bare(None);
    }
    if bytes.starts_with(b"--watch=") {
        return WatchFlag::Inline(None);
    }
    let Some(pos) = short_watch_pos(bytes, value_less_short_flags) else {
        return WatchFlag::Other;
    };
    // Everything after `w` is the path attached to it; only the preceding valueless short flags
    // are kept. The prefix is ASCII because it consists only of recognized option bytes.
    let kept = (pos > 1).then(|| OsString::from(std::str::from_utf8(&bytes[..pos]).unwrap()));
    if bytes.len() == pos + 1 { WatchFlag::Bare(kept) } else { WatchFlag::Inline(kept) }
}

/// Returns the byte index after `w` in a short flag cluster, if `arg` contains a watch flag.
///
/// clap parses a short flag cluster from left to right. Only short flags with no value in the
/// active command may precede `w`; a value-taking flag such as `-D` or `-C` makes the remaining
/// bytes its value instead.
fn short_watch_pos(arg: &[u8], value_less_short_flags: &HashSet<char>) -> Option<usize> {
    let cluster = arg.strip_prefix(b"-")?;
    if cluster.is_empty() || cluster.starts_with(b"-") {
        return None;
    }
    let pos = cluster.iter().position(|&byte| byte == b'w')?;
    cluster[..pos]
        .iter()
        .all(|&byte| byte.is_ascii() && value_less_short_flags.contains(&(byte as char)))
        .then_some(pos + 1)
}

/// Returns whether `arg` looks like an option, which is where clap stops consuming the values of a
/// `num_args(0..)` argument. A lone `-` is a value.
fn looks_like_option(arg: &OsStr) -> bool {
    let bytes = arg.as_encoded_bytes();
    bytes.len() > 1 && bytes.starts_with(b"-")
}

/// Returns the valueless short flags supported by the active command and its global options.
fn value_less_short_flags(cmd_args: &[OsString]) -> HashSet<char> {
    fn collect_flags(
        command: &ClapCommand,
        matches: &clap::ArgMatches,
        flags: &mut HashSet<char>,
        is_root: bool,
    ) {
        for arg in command.get_arguments() {
            if (!is_root || arg.is_global_set())
                && matches!(
                    arg.get_action(),
                    ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
                )
            {
                flags.extend(arg.get_short());
                flags.extend(arg.get_all_short_aliases().unwrap_or_default());
            }
        }

        if let Some((name, sub_matches)) = matches.subcommand()
            && let Some(subcommand) = command.find_subcommand(name)
        {
            collect_flags(subcommand, sub_matches, flags, false);
        }
    }

    let command = Forge::command();
    let Ok(matches) = command.clone().try_get_matches_from(cmd_args.to_vec()) else {
        return HashSet::default();
    };
    let mut flags = HashSet::default();
    collect_flags(&command, &matches, &mut flags, true);
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use watchexec_events::Modifiers;

    #[cfg(unix)]
    use std::os::unix::ffi::OsStrExt;

    fn key_event(key: char, modifiers: Modifiers) -> Event {
        Event {
            tags: vec![Tag::Keyboard(Keyboard::Key { key: KeyCode::Char(key), modifiers })],
            ..Default::default()
        }
    }

    fn clean(args: &[&str]) -> Vec<OsString> {
        clean_cmd_args(args.iter().copied().map(OsString::from).collect())
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
        assert_eq!(clean(&["forge", "build", "-vw"]), ["forge", "build", "-v"]);
    }

    #[test]
    fn locked_survives_cleaning_watch_args() {
        assert_eq!(
            clean(&["forge", "build", "--locked", "--watch", "src", "test"]),
            ["forge", "build", "--locked"]
        );

        assert_eq!(
            clean(&["forge", "build", "--watch", "src", "--locked"]),
            ["forge", "build", "--locked"]
        );
    }

    #[test]
    fn watch_spellings_survive_cleaning_watch_args() {
        assert_eq!(clean(&["forge", "test", "--watch", "src"]), ["forge", "test"]);
        assert_eq!(clean(&["forge", "test", "--watch=src"]), ["forge", "test"]);
        assert_eq!(clean(&["forge", "test", "-w", "src"]), ["forge", "test"]);
        assert_eq!(clean(&["forge", "test", "-wsrc"]), ["forge", "test"]);
        assert_eq!(clean(&["forge", "test", "-vw", "src"]), ["forge", "test", "-v"]);
        assert_eq!(clean(&["forge", "test", "-vwsrc"]), ["forge", "test", "-v"]);
    }

    #[test]
    fn repeated_watch_survives_cleaning_watch_args() {
        assert_eq!(
            clean(&["forge", "test", "--watch", "src", "--watch", "test"]),
            ["forge", "test"]
        );

        assert_eq!(clean(&["forge", "test", "--watch=src", "-w"]), ["forge", "test"]);
        assert_eq!(clean(&["forge", "test", "-w", "--watch=src"]), ["forge", "test"]);
    }

    #[test]
    fn other_flags_survive_cleaning_watch_args() {
        // `--watch-delay` is a different flag.
        assert_eq!(
            clean(&["forge", "test", "--watch-delay", "1s"]),
            ["forge", "test", "--watch-delay", "1s"]
        );

        // A `w` in the value attached to another short flag is not the watch flag.
        assert_eq!(clean(&["forge", "test", "-C./workspace"]), ["forge", "test", "-C./workspace"]);

        assert_eq!(clean(&["forge", "build", "-Dwarnings"]), ["forge", "build", "-Dwarnings"]);
    }

    #[test]
    fn command_specific_short_flags_survive_cleaning_watch_args() {
        assert_eq!(clean(&["forge", "build", "-qw", "src"]), ["forge", "build", "-q"]);
        assert_eq!(clean(&["forge", "fmt", "-rw", "src"]), ["forge", "fmt", "-r"]);
    }

    #[test]
    fn watch_cleanup_stops_after_double_dash() {
        assert_eq!(
            clean(&["forge", "fmt", "--watch", "src", "--", "--watch=keep.sol"]),
            ["forge", "fmt", "--", "--watch=keep.sol"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_watch_survives_cleaning_watch_args() {
        let mut args = ["forge", "test", "--watch"].map(OsString::from).to_vec();
        args.push(OsStr::from_bytes(b"src/\xff").to_os_string());

        assert_eq!(clean_cmd_args(args), ["forge", "test"]);
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_watch_suffixes_and_options_are_preserved() {
        let args = ["forge", "fmt"]
            .map(OsString::from)
            .into_iter()
            .chain([OsStr::from_bytes(b"--watch=src/\xff").to_os_string()])
            .collect();
        assert_eq!(clean_cmd_args(args), ["forge", "fmt"]);

        let args = ["forge", "fmt"]
            .map(OsString::from)
            .into_iter()
            .chain([OsStr::from_bytes(b"-wsrc/\xff").to_os_string()])
            .collect();
        assert_eq!(clean_cmd_args(args), ["forge", "fmt"]);

        let args = ["forge", "build", "-w", "src"]
            .map(OsString::from)
            .into_iter()
            .chain([OsStr::from_bytes(b"--out=out/\xff").to_os_string()])
            .collect();
        assert_eq!(
            clean_cmd_args(args),
            vec![
                OsString::from("forge"),
                OsString::from("build"),
                OsStr::from_bytes(b"--out=out/\xff").to_os_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_program_path_is_preserved() {
        let args = vec![OsStr::from_bytes(b"forge\xff").to_os_string(), OsString::from("build")];
        let command = watch_command(args).unwrap();
        let Program::Exec { prog, args } = command.program else { panic!("expected exec program") };
        assert_eq!(prog, PathBuf::from(OsStr::from_bytes(b"forge\xff")));
        assert_eq!(args, ["build"]);
    }
}
