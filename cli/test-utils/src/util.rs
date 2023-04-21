use ethers_solc::{
    cache::SolFilesCache,
    project_util::{copy_dir, TempProject},
    ArtifactOutput, ConfigurableArtifacts, PathStyle, ProjectPathsConfig, Solc,
};
use eyre::WrapErr;
use foundry_config::Config;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use std::{
    env,
    ffi::OsStr,
    fmt::Display,
    fs,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static CURRENT_DIR_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// A lock used for pre-installing commonly used solc versions once.
/// Pre-installing is useful, because if two forge test require a missing solc at the same time, one
/// can encounter an OS error 26 textfile busy if it tries to write the freshly downloaded solc to
/// the right location while the other test already did that and is currently executing this solc
/// binary.
static PRE_INSTALL_SOLC_LOCK: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

// This stores `true` if the current terminal is a tty
pub static IS_TTY: Lazy<bool> = Lazy::new(|| is_terminal::is_terminal(&std::io::stdout()));

/// Contains a `forge init` initialized project
pub static FORGE_INITIALIZED: Lazy<TestProject> = Lazy::new(|| {
    let (prj, mut cmd) = setup_forge("init-template", PathStyle::Dapptools);
    cmd.args(["init", "--force"]);
    cmd.assert_non_empty_stdout();
    prj
});

// identifier for tests
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Copies an initialized project to the given path
pub fn initialize(target: impl AsRef<Path>) {
    FORGE_INITIALIZED.copy_to(target)
}

/// Clones a remote repository into the specified directory.
pub fn clone_remote(
    repo_url: &str,
    target_dir: impl AsRef<Path>,
) -> std::io::Result<process::Output> {
    Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--recursive",
            repo_url,
            target_dir.as_ref().to_str().expect("Target path for git clone does not exist"),
        ])
        .output()
}

/// Setup an empty test project and return a command pointing to the forge
/// executable whose CWD is set to the project's root.
///
/// The name given will be used to create the directory. Generally, it should
/// correspond to the test name.
#[track_caller]
pub fn setup_forge(name: &str, style: PathStyle) -> (TestProject, TestCommand) {
    setup_forge_project(TestProject::new(name, style))
}

pub fn setup_forge_project(test: TestProject) -> (TestProject, TestCommand) {
    // preinstall commonly used solc once, we execute this here because this is the shared
    // entrypoint used by all `forgetest!` macros
    install_commonly_used_solc();

    let cmd = test.forge_command();
    (test, cmd)
}

/// How to initialize a remote git project
#[derive(Debug, Clone)]
pub struct RemoteProject {
    id: String,
    run_build: bool,
    run_commands: Vec<Vec<String>>,
    path_style: PathStyle,
}

impl RemoteProject {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            run_build: true,
            run_commands: vec![],
            path_style: PathStyle::Dapptools,
        }
    }

    /// Whether to run `forge build`
    pub fn set_build(mut self, run_build: bool) -> Self {
        self.run_build = run_build;
        self
    }

    /// Configures the project's pathstyle
    pub fn path_style(mut self, path_style: PathStyle) -> Self {
        self.path_style = path_style;
        self
    }

    /// Add another command to run after cloning
    pub fn cmd(mut self, cmd: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.run_commands.push(cmd.into_iter().map(Into::into).collect());
        self
    }
}

impl<T: Into<String>> From<T> for RemoteProject {
    fn from(id: T) -> Self {
        Self::new(id)
    }
}

/// Setups a new local forge project by cloning and initializing the `RemoteProject`
///
/// This will
///   1. clone the prj, like "transmissions1/solmate"
///   2. run `forge build`, if configured
///   3. run additional commands
///
/// # Panics
///
/// If anything goes wrong during, checkout, build, or other commands are unsuccessful
pub fn setup_forge_remote(prj: impl Into<RemoteProject>) -> (TestProject, TestCommand) {
    try_setup_forge_remote(prj).unwrap()
}

/// Same as `setup_forge_remote` but not panicing
pub fn try_setup_forge_remote(
    config: impl Into<RemoteProject>,
) -> eyre::Result<(TestProject, TestCommand)> {
    let config = config.into();
    let mut tmp = TempProject::checkout(&config.id).wrap_err("failed to checkout project")?;
    tmp.project_mut().paths = config.path_style.paths(tmp.root())?;

    let prj = TestProject::with_project(tmp);
    if config.run_build {
        let mut cmd = prj.forge_command();
        cmd.arg("build");
        cmd.ensure_execute_success().wrap_err("`forge build` unsuccessful")?;
    }
    for addon in config.run_commands {
        debug_assert!(!addon.is_empty());
        let mut cmd = Command::new(&addon[0]);
        if addon.len() > 1 {
            cmd.args(&addon[1..]);
        }
        let status = cmd
            .current_dir(prj.root())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .wrap_err_with(|| format!("Failed to execute {addon:?}"))?;
        eyre::ensure!(status.success(), "Failed to execute command {:?}", addon);
    }

    let cmd = prj.forge_command();
    Ok((prj, cmd))
}

pub fn setup_cast(name: &str, style: PathStyle) -> (TestProject, TestCommand) {
    setup_cast_project(TestProject::new(name, style))
}

pub fn setup_cast_project(test: TestProject) -> (TestProject, TestCommand) {
    let cmd = test.cast_command();
    (test, cmd)
}

/// pre-installs commonly used solc versions
fn install_commonly_used_solc() {
    let mut is_preinstalled = PRE_INSTALL_SOLC_LOCK.lock();
    if !*is_preinstalled {
        let v0_8_10 = std::thread::spawn(|| Solc::blocking_install(&"0.8.10".parse().unwrap()));
        let v0_8_13 = std::thread::spawn(|| Solc::blocking_install(&"0.8.13".parse().unwrap()));

        let wait = |res: std::thread::JoinHandle<_>| -> Result<(), ()> {
            if let Err(err) = res.join().unwrap() {
                eprintln!("{err:?}");
                // there could be another process that's currently installing this version, so we
                // sleep here for a bit and assume the other process will be finished then
                std::thread::sleep(std::time::Duration::from_secs(15));
                Err(())
            } else {
                Ok(())
            }
        };

        // only set to installed if succeeded
        *is_preinstalled = wait(v0_8_10).and(wait(v0_8_13)).is_ok();
    }
}

/// `TestProject` represents a temporary project to run tests against.
///
/// Test projects are created from a global atomic counter to avoid duplicates.
#[derive(Clone, Debug)]
pub struct TestProject<T: ArtifactOutput = ConfigurableArtifacts> {
    /// The directory in which this test executable is running.
    root: PathBuf,
    /// The project in which the test should run.
    inner: Arc<TempProject<T>>,
}

impl TestProject {
    /// Create a new test project with the given name. The name
    /// does not need to be distinct for each invocation, but should correspond
    /// to a logical grouping of tests.
    pub fn new(name: &str, style: PathStyle) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let project = pretty_err(name, TempProject::with_style(&format!("{name}-{id}"), style));
        Self::with_project(project)
    }

    pub fn with_project(project: TempProject) -> Self {
        let root =
            env::current_exe().unwrap().parent().expect("executable's directory").to_path_buf();
        Self { root, inner: Arc::new(project) }
    }

    /// Returns the root path of the project's workspace.
    pub fn root(&self) -> &Path {
        self.inner.root()
    }

    pub fn inner(&self) -> &TempProject {
        &self.inner
    }

    pub fn paths(&self) -> &ProjectPathsConfig {
        self.inner().paths()
    }

    /// Returns the path to the project's `foundry.toml` file
    pub fn config_path(&self) -> PathBuf {
        self.root().join(Config::FILE_NAME)
    }

    /// Returns the path to the project's cache file
    pub fn cache_path(&self) -> &PathBuf {
        &self.paths().cache
    }

    /// Writes the given config as toml to `foundry.toml`
    pub fn write_config(&self, config: Config) {
        let file = self.config_path();
        pretty_err(&file, fs::write(&file, config.to_string_pretty().unwrap()));
    }

    /// Asserts that the `<root>/foundry.toml` file exits
    pub fn assert_config_exists(&self) {
        assert!(self.config_path().exists());
    }

    /// Asserts that the `<root>/cache/sol-files-cache.json` file exits
    pub fn assert_cache_exists(&self) {
        assert!(self.cache_path().exists());
    }

    /// Asserts that the `<root>/out` file exits
    pub fn assert_artifacts_dir_exists(&self) {
        assert!(self.paths().artifacts.exists());
    }

    /// Creates all project dirs and ensure they were created
    pub fn assert_create_dirs_exists(&self) {
        self.paths().create_all().unwrap_or_else(|_| panic!("Failed to create project paths"));
        SolFilesCache::default().write(&self.paths().cache).expect("Failed to create cache");
        self.assert_all_paths_exist();
    }

    /// Ensures that the given layout exists
    pub fn assert_style_paths_exist(&self, style: PathStyle) {
        let paths = style.paths(&self.paths().root).unwrap();
        config_paths_exist(&paths, self.inner().project().cached);
    }

    /// Copies the project's root directory to the given target
    pub fn copy_to(&self, target: impl AsRef<Path>) {
        let target = target.as_ref();
        pretty_err(target, fs::create_dir_all(target));
        pretty_err(target, copy_dir(self.root(), target));
    }

    /// Creates a file with contents `contents` in the test project's directory. The
    /// file will be deleted when the project is dropped.
    pub fn create_file(&self, path: impl AsRef<Path>, contents: &str) -> PathBuf {
        let path = path.as_ref();
        if !path.is_relative() {
            panic!("create_file(): file path is absolute");
        }
        let path = self.root().join(path);
        if let Some(parent) = path.parent() {
            pretty_err(parent, std::fs::create_dir_all(parent));
        }
        let file = pretty_err(&path, File::create(&path));
        let mut writer = BufWriter::new(file);
        pretty_err(&path, writer.write_all(contents.as_bytes()));
        path
    }

    /// Adds DSTest as a source under "test.sol"
    pub fn insert_ds_test(&self) -> PathBuf {
        let s = include_str!("../../../testdata/lib/ds-test/src/test.sol");
        self.inner().add_source("test.sol", s).unwrap()
    }

    /// Adds `console.sol` as a source under "console.sol"
    pub fn insert_console(&self) -> PathBuf {
        let s = include_str!("../../../testdata/logs/console.sol");
        self.inner().add_source("console.sol", s).unwrap()
    }

    /// Asserts all project paths exist
    ///
    ///   - sources
    ///   - artifacts
    ///   - libs
    ///   - cache
    pub fn assert_all_paths_exist(&self) {
        let paths = self.paths();
        config_paths_exist(paths, self.inner().project().cached);
    }

    /// Asserts that the artifacts dir and cache don't exist
    pub fn assert_cleaned(&self) {
        let paths = self.paths();
        assert!(!paths.cache.exists());
        assert!(!paths.artifacts.exists());
    }

    /// Creates a new command that is set to use the forge executable for this project
    #[track_caller]
    pub fn forge_command(&self) -> TestCommand {
        let cmd = self.forge_bin();
        let _lock = CURRENT_DIR_LOCK.lock();
        TestCommand {
            project: self.clone(),
            cmd,
            current_dir_lock: None,
            saved_cwd: pretty_err("<current dir>", std::env::current_dir()),
            stdin_fun: None,
        }
    }

    /// Creates a new command that is set to use the cast executable for this project
    pub fn cast_command(&self) -> TestCommand {
        let mut cmd = self.cast_bin();
        cmd.current_dir(self.inner.root());
        let _lock = CURRENT_DIR_LOCK.lock();
        TestCommand {
            project: self.clone(),
            cmd,
            current_dir_lock: None,
            saved_cwd: pretty_err("<current dir>", std::env::current_dir()),
            stdin_fun: None,
        }
    }

    /// Returns the path to the forge executable.
    pub fn forge_bin(&self) -> process::Command {
        let forge = self.root.join(format!("../forge{}", env::consts::EXE_SUFFIX));
        let mut cmd = process::Command::new(forge);
        cmd.current_dir(self.inner.root());
        cmd.env("NO_COLOR", "1");
        cmd
    }

    /// Returns the path to the cast executable.
    pub fn cast_bin(&self) -> process::Command {
        let cast = self.root.join(format!("../cast{}", env::consts::EXE_SUFFIX));
        let mut cmd = process::Command::new(cast);
        cmd.env("NO_COLOR", "1");
        cmd
    }

    /// Returns the `Config` as spit out by `forge config`
    pub fn config_from_output<I, A>(&self, args: I) -> Config
    where
        I: IntoIterator<Item = A>,
        A: AsRef<OsStr>,
    {
        let mut cmd = self.forge_bin();
        cmd.arg("config").arg("--root").arg(self.root()).args(args).arg("--json");
        let output = cmd.output().unwrap();
        let c = String::from_utf8_lossy(&output.stdout);
        let config: Config = serde_json::from_str(c.as_ref()).unwrap();
        config.sanitized()
    }

    /// Removes all files and dirs inside the project's root dir
    pub fn wipe(&self) {
        pretty_err(self.root(), fs::remove_dir_all(self.root()));
        pretty_err(self.root(), fs::create_dir_all(self.root()));
    }

    /// Removes all contract files from `src`, `test`, `script`
    pub fn wipe_contracts(&self) {
        fn rm_create(path: &Path) {
            pretty_err(path, fs::remove_dir_all(path));
            pretty_err(path, fs::create_dir(path));
        }
        rm_create(&self.paths().sources);
        rm_create(&self.paths().tests);
        rm_create(&self.paths().scripts);
    }
}

impl Drop for TestCommand {
    fn drop(&mut self) {
        let _lock = self.current_dir_lock.take().unwrap_or_else(|| CURRENT_DIR_LOCK.lock());
        if self.saved_cwd.exists() {
            let _ = std::env::set_current_dir(&self.saved_cwd);
        }
    }
}

fn config_paths_exist(paths: &ProjectPathsConfig, cached: bool) {
    if cached {
        assert!(paths.cache.exists());
    }
    assert!(paths.sources.exists());
    assert!(paths.artifacts.exists());
    paths.libraries.iter().for_each(|lib| assert!(lib.exists()));
}

#[track_caller]
pub fn pretty_err<T, E: std::error::Error>(path: impl AsRef<Path>, res: Result<T, E>) -> T {
    match res {
        Ok(t) => t,
        Err(err) => panic!("{}: {err:?}", path.as_ref().display()),
    }
}

pub fn read_string(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    pretty_err(path, std::fs::read_to_string(path))
}

/// A simple wrapper around a process::Command with some conveniences.
pub struct TestCommand {
    saved_cwd: PathBuf,
    /// The project used to launch this command.
    project: TestProject,
    /// The actual command we use to control the process.
    cmd: Command,
    // initial: Command,
    current_dir_lock: Option<parking_lot::lock_api::MutexGuard<'static, parking_lot::RawMutex, ()>>,
    stdin_fun: Option<Box<dyn FnOnce(process::ChildStdin)>>,
}

impl TestCommand {
    /// Returns a mutable reference to the underlying command.
    pub fn cmd(&mut self) -> &mut Command {
        &mut self.cmd
    }

    /// replaces the command
    pub fn set_cmd(&mut self, cmd: Command) -> &mut TestCommand {
        self.cmd = cmd;
        self
    }

    /// Resets the command
    pub fn forge_fuse(&mut self) -> &mut TestCommand {
        self.set_cmd(self.project.forge_bin())
    }

    pub fn cast_fuse(&mut self) -> &mut TestCommand {
        self.set_cmd(self.project.cast_bin())
    }

    /// Sets the current working directory
    pub fn set_current_dir(&mut self, p: impl AsRef<Path>) {
        drop(self.current_dir_lock.take());
        let lock = CURRENT_DIR_LOCK.lock();
        self.current_dir_lock = Some(lock);
        let p = p.as_ref();
        pretty_err(p, std::env::set_current_dir(p));
    }

    /// Add an argument to pass to the command.
    pub fn arg<A: AsRef<OsStr>>(&mut self, arg: A) -> &mut TestCommand {
        self.cmd.arg(arg);
        self
    }

    /// Add any number of arguments to the command.
    pub fn args<I, A>(&mut self, args: I) -> &mut TestCommand
    where
        I: IntoIterator<Item = A>,
        A: AsRef<OsStr>,
    {
        self.cmd.args(args);
        self
    }

    pub fn stdin(&mut self, fun: impl FnOnce(process::ChildStdin) + 'static) -> &mut TestCommand {
        self.stdin_fun = Some(Box::new(fun));
        self
    }

    /// Convenience function to add `--root project.root()` argument
    pub fn root_arg(&mut self) -> &mut TestCommand {
        let root = self.project.root().to_path_buf();
        self.arg("--root").arg(root)
    }

    /// Set the environment variable `k` to value `v` for the command.
    pub fn set_env(&mut self, k: impl AsRef<OsStr>, v: impl Display) {
        self.cmd.env(k, v.to_string());
    }

    /// Unsets the environment variable `k` for the command.
    pub fn unset_env(&mut self, k: impl AsRef<OsStr>) {
        self.cmd.env_remove(k);
    }

    /// Set the working directory for this command.
    ///
    /// Note that this does not need to be called normally, since the creation
    /// of this TestCommand causes its working directory to be set to the
    /// test's directory automatically.
    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut TestCommand {
        self.cmd.current_dir(dir);
        self
    }

    /// Returns the `Config` as spit out by `forge config`
    pub fn config(&mut self) -> Config {
        self.cmd.args(["config", "--json"]);
        let output = self.output();
        let c = String::from_utf8_lossy(&output.stdout);
        let config = serde_json::from_str(c.as_ref()).unwrap();
        self.forge_fuse();
        config
    }

    /// Runs `git init` inside the project's dir
    pub fn git_init(&self) -> process::Output {
        let mut cmd = Command::new("git");
        cmd.arg("init").current_dir(self.project.root());
        let output = cmd.output().unwrap();
        self.expect_success(output)
    }

    /// Runs and captures the stdout of the given command.
    pub fn stdout(&mut self) -> String {
        let o = self.output();
        let stdout = String::from_utf8_lossy(&o.stdout);
        match stdout.parse::<String>() {
            Ok(t) => t.replace("\r\n", "\n"),
            Err(err) => {
                panic!("could not convert from string: {err:?}\n\n{stdout}");
            }
        }
    }

    /// Returns the `stderr` of the output as `String`.
    pub fn stderr_lossy(&mut self) -> String {
        let output = self.execute();
        String::from_utf8_lossy(&output.stderr).to_string().replace("\r\n", "\n")
    }

    /// Returns the `stdout` of the output as `String`.
    pub fn stdout_lossy(&mut self) -> String {
        String::from_utf8_lossy(&self.output().stdout).to_string().replace("\r\n", "\n")
    }

    /// Returns the output but does not expect that the command was successful
    pub fn unchecked_output(&mut self) -> process::Output {
        self.execute()
    }

    /// Gets the output of a command. If the command failed, then this panics.
    pub fn output(&mut self) -> process::Output {
        let output = self.execute();
        self.expect_success(output)
    }

    /// Runs the command and asserts that it resulted in success
    pub fn assert_success(&mut self) {
        self.output();
    }

    /// Executes command, applies stdin function and returns output
    pub fn execute(&mut self) -> process::Output {
        self.try_execute().unwrap()
    }

    pub fn try_execute(&mut self) -> std::io::Result<process::Output> {
        let mut child =
            self.cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::piped()).spawn()?;
        if let Some(fun) = self.stdin_fun.take() {
            fun(child.stdin.take().unwrap())
        }
        child.wait_with_output()
    }

    /// Executes command and expects an successful result
    #[track_caller]
    pub fn ensure_execute_success(&mut self) -> eyre::Result<process::Output> {
        let out = self.try_execute()?;
        self.ensure_success(out)
    }

    /// Runs the command and prints its output
    /// You have to pass --nocapture to cargo test or the print won't be displayed.
    /// The full command would be: cargo test -- --nocapture
    pub fn print_output(&mut self) {
        let output = self.execute();
        println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    /// Writes the content of the output to new fixture files
    pub fn write_fixtures(&mut self, name: impl AsRef<Path>) {
        let name = name.as_ref();
        if let Some(parent) = name.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let output = self.execute();
        fs::write(format!("{}.stdout", name.display()), &output.stdout).unwrap();
        fs::write(format!("{}.stderr", name.display()), &output.stderr).unwrap();
    }

    /// Runs the command and asserts that it resulted in an error exit code.
    #[track_caller]
    pub fn assert_err(&mut self) {
        let o = self.execute();
        if o.status.success() {
            panic!(
                "\n\n===== {:?} =====\n\
                 command succeeded but expected failure!\
                 \n\ncwd: {}\
                 \n\nstatus: {}\
                 \n\nstdout: {}\n\nstderr: {}\
                 \n\n=====\n",
                self.cmd,
                self.project.inner.paths(),
                o.status,
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }

    /// Runs the command and asserts that something was printed to stderr.
    #[track_caller]
    pub fn assert_non_empty_stderr(&mut self) {
        let o = self.execute();
        if o.status.success() || o.stderr.is_empty() {
            panic!(
                "\n\n===== {:?} =====\n\
                 command succeeded but expected failure!\
                 \n\ncwd: {}\
                 \n\nstatus: {}\
                 \n\nstdout: {}\n\nstderr: {}\
                 \n\n=====\n",
                self.cmd,
                self.project.inner.paths(),
                o.status,
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }

    /// Runs the command and asserts that something was printed to stdout.
    #[track_caller]
    pub fn assert_non_empty_stdout(&mut self) {
        let o = self.execute();
        if !o.status.success() || o.stdout.is_empty() {
            panic!(
                "\n\n===== {:?} =====\n\
                 command failed but expected success!\
                 \n\ncwd: {}\
                 \n\nstatus: {}\
                 \n\nstdout: {}\n\nstderr: {}\
                 \n\n=====\n",
                self.cmd,
                self.project.inner.paths(),
                o.status,
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }

    /// Runs the command and asserts that nothing was printed to stdout.
    #[track_caller]
    pub fn assert_empty_stdout(&mut self) {
        let o = self.execute();
        if !o.status.success() || !o.stderr.is_empty() {
            panic!(
                "\n\n===== {:?} =====\n\
                 command succeeded but expected failure!\
                 \n\ncwd: {}\
                 \n\nstatus: {}\
                 \n\nstdout: {}\n\nstderr: {}\
                 \n\n=====\n",
                self.cmd,
                self.project.inner.paths(),
                o.status,
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
        }
    }

    #[track_caller]
    fn expect_success(&self, out: process::Output) -> process::Output {
        self.ensure_success(out).unwrap()
    }

    #[track_caller]
    pub fn ensure_success(&self, out: process::Output) -> eyre::Result<process::Output> {
        if !out.status.success() {
            let suggest = if out.stderr.is_empty() {
                "\n\nDid your forge command end up with no output?".to_string()
            } else {
                "".to_string()
            };
            eyre::bail!(
                "\n\n==========\n\
                    command failed but expected success!\
                    {}\
                    \n\ncommand: {:?}\
                    \n\ncwd: {}\
                    \n\nstatus: {}\
                    \n\nstdout: {}\
                    \n\nstderr: {}\
                    \n\n==========\n",
                suggest,
                self.cmd,
                self.project.inner.paths(),
                out.status,
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(out)
    }
}

/// Extension trait for `std::process::Output`
///
/// These function will read the path's content and assert that the process' output matches the
/// fixture. Since `forge` commands may emit colorized output depending on whether the current
/// terminal is tty, the path argument can be wrapped in [tty_fixture_path()]
pub trait OutputExt {
    /// Ensure the command wrote the expected data to `stdout`.
    fn stdout_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self;

    /// Ensure the command wrote the expected data to `stderr`.
    fn stderr_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self;
}

/// Patterns to remove from fixtures before comparing output
///
/// This should strip everything that can vary from run to run, like elapsed time, file paths
static IGNORE_IN_FIXTURES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\r|finished in (.*)?s|-->(.*).sol|Location(.|\n)*\.rs(.|\n)*Backtrace|Installing solc version(.*?)\n|Successfully installed solc(.*?)\n|runs: \d+, μ: \d+, ~: \d+)").unwrap()
});

impl OutputExt for process::Output {
    #[track_caller]
    fn stdout_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self {
        let expected = fs::read_to_string(expected_path).unwrap();
        let expected = IGNORE_IN_FIXTURES.replace_all(&expected, "").replace('\\', "/");
        let stdout = String::from_utf8_lossy(&self.stdout);
        let out = IGNORE_IN_FIXTURES.replace_all(&stdout, "").replace('\\', "/");

        pretty_assertions::assert_eq!(expected, out);

        self
    }

    #[track_caller]
    fn stderr_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self {
        let expected = fs::read_to_string(expected_path).unwrap();
        let expected = IGNORE_IN_FIXTURES.replace_all(&expected, "").replace('\\', "/");
        let stderr = String::from_utf8_lossy(&self.stderr);
        let out = IGNORE_IN_FIXTURES.replace_all(&stderr, "").replace('\\', "/");

        pretty_assertions::assert_eq!(expected, out);
        self
    }
}

/// Returns the fixture path depending on whether the current terminal is tty
///
/// This is useful in combination with [OutputExt]
pub fn tty_fixture_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if *IS_TTY {
        return if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            path.with_extension(format!("tty.{ext}"))
        } else {
            path.with_extension("tty")
        }
    }
    path.to_path_buf()
}

/// Return a recursive listing of all files and directories in the given
/// directory. This is useful for debugging transient and odd failures in
/// integration tests.
pub fn dir_list<P: AsRef<Path>>(dir: P) -> Vec<String> {
    walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .map(|result| result.unwrap().path().to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_path_works() {
        let path = "tests/fixture/test.stdout";
        if *IS_TTY {
            assert_eq!(tty_fixture_path(path), PathBuf::from("tests/fixture/test.tty.stdout"));
        } else {
            assert_eq!(tty_fixture_path(path), PathBuf::from(path));
        }
    }

    #[test]
    fn fixture_regex_matches() {
        assert!(IGNORE_IN_FIXTURES.is_match(
            r#"
Location:
   [35mcli/src/compile.rs[0m:[35m151[0m

Backtrace omitted.
        "#
        ));
    }
}
