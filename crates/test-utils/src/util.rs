use crate::init_tracing;
use eyre::{Result, WrapErr};
use fd_lock::RwLock;
use foundry_compilers::{
    cache::SolFilesCache,
    error::Result as SolcResult,
    project_util::{copy_dir, TempProject},
    ArtifactOutput, ConfigurableArtifacts, PathStyle, ProjectPathsConfig,
};
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
    io::{BufWriter, IsTerminal, Write},
    path::{Path, PathBuf},
    process::{ChildStdin, Command, Output, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

static CURRENT_DIR_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Stores whether `stdout` is a tty / terminal.
pub static IS_TTY: Lazy<bool> = Lazy::new(|| std::io::stdout().is_terminal());

/// Global default template path. Contains the global template project from which all other
/// temp projects are initialized. See [`initialize()`] for more info.
static TEMPLATE_PATH: Lazy<PathBuf> =
    Lazy::new(|| env::temp_dir().join("foundry-forge-test-template"));

/// Global default template lock. If its contents are not exactly `"1"`, the global template will
/// be re-initialized. See [`initialize()`] for more info.
static TEMPLATE_LOCK: Lazy<PathBuf> =
    Lazy::new(|| env::temp_dir().join("foundry-forge-test-template.lock"));

/// Global test identifier.
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// The default Solc version used when compiling tests.
pub const SOLC_VERSION: &str = "0.8.23";

/// Another Solc version used when compiling tests. Necessary to avoid downloading multiple
/// versions.
pub const OTHER_SOLC_VERSION: &str = "0.8.22";

/// Creates a file lock to the global template dir.
pub fn template_lock() -> RwLock<File> {
    let lock_path = &*TEMPLATE_LOCK;
    let lock_file = pretty_err(
        lock_path,
        fs::OpenOptions::new().read(true).write(true).create(true).open(lock_path),
    );
    RwLock::new(lock_file)
}

/// Initializes a project with `forge init` at the given path.
///
/// This should be called after an empty project is created like in
/// [some of this crate's macros](crate::forgetest_init).
///
/// ## Note
///
/// This doesn't always run `forge init`, instead opting to copy an already-initialized template
/// project from a global template path. This is done to speed up tests.
///
/// This used to use a `static` [`Lazy`], but this approach does not with `cargo-nextest` because it
/// runs each test in a separate process. Instead, we use a global lock file to ensure that only one
/// test can initialize the template at a time.
pub fn initialize(target: &Path) {
    eprintln!("initializing {}", target.display());

    let tpath = TEMPLATE_PATH.as_path();
    pretty_err(tpath, fs::create_dir_all(tpath));

    // Initialize the global template if necessary.
    let mut lock = template_lock();
    let mut _read = Some(lock.read().unwrap());
    if fs::read_to_string(&*TEMPLATE_LOCK).unwrap() != "1" {
        // We are the first to acquire the lock:
        // - initialize a new empty temp project;
        // - run `forge init`;
        // - run `forge build`;
        // - copy it over to the global template;
        // Ideally we would be able to initialize a temp project directly in the global template,
        // but `TempProject` does not currently allow this: https://github.com/foundry-rs/compilers/issues/22

        // Release the read lock and acquire a write lock, initializing the lock file.
        _read = None;
        let mut write = lock.write().unwrap();
        write.write_all(b"1").unwrap();

        // Initialize and build.
        let (prj, mut cmd) = setup_forge("template", foundry_compilers::PathStyle::Dapptools);
        eprintln!("- initializing template dir in {}", prj.root().display());

        cmd.args(["init", "--force"]).assert_success();
        cmd.forge_fuse().args(["build", "--use", SOLC_VERSION]).assert_success();

        // Remove the existing template, if any.
        let _ = fs::remove_dir_all(tpath);

        // Copy the template to the global template path.
        pretty_err(tpath, copy_dir(prj.root(), tpath));

        // Release the write lock and acquire a new read lock.
        drop(write);
        _read = Some(lock.read().unwrap());
    }

    eprintln!("- copying template dir from {}", tpath.display());
    pretty_err(target, fs::create_dir_all(target));
    pretty_err(target, copy_dir(tpath, target));
}

/// Clones a remote repository into the specified directory. Panics if the command fails.
pub fn clone_remote(repo_url: &str, target_dir: &str) {
    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth=1", "--recursive", "--shallow-submodules", repo_url, target_dir]);
    eprintln!("{cmd:?}");
    let status = cmd.status().unwrap();
    if !status.success() {
        panic!("git clone failed: {status:?}");
    }
    eprintln!();
}

/// Runs common installation commands, such as `make` and `npm`. Continues if any command fails.
pub fn run_install_commands(root: &Path) {
    let root_files =
        std::fs::read_dir(root).unwrap().flatten().map(|x| x.path()).collect::<Vec<_>>();
    let contains = |path: &str| root_files.iter().any(|p| p.to_str().unwrap().contains(path));
    let run = |args: &[&str]| {
        let mut cmd = Command::new(args[0]);
        cmd.args(&args[1..]).current_dir(root);
        eprintln!("cd {}; {cmd:?}", root.display());
        #[cfg(windows)]
        let st = cmd.status();
        #[cfg(not(windows))]
        let st = cmd.status().unwrap();
        eprintln!("\n\n{cmd:?}: {st:?}");
    };
    let maybe_run = |path: &str, args: &[&str]| {
        let c = contains(path);
        if c {
            run(args);
        }
        c
    };

    maybe_run("Makefile", &["make", "install"]);
    let pnpm = maybe_run("pnpm-lock.yaml", &["pnpm", "install", "--prefer-offline"]);
    let yarn = maybe_run("yarn.lock", &["yarn", "install", "--prefer-offline"]);
    if !pnpm && !yarn && contains("package.json") {
        run(&["npm", "install"]);
    }
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
) -> Result<(TestProject, TestCommand)> {
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

/// `TestProject` represents a temporary project to run tests against.
///
/// Test projects are created from a global atomic counter to avoid duplicates.
#[derive(Clone, Debug)]
pub struct TestProject<T: ArtifactOutput = ConfigurableArtifacts> {
    /// The directory in which this test executable is running.
    exe_root: PathBuf,
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
        init_tracing();
        let this = env::current_exe().unwrap();
        let exe_root = this.parent().expect("executable's directory").to_path_buf();
        Self { exe_root, inner: Arc::new(project) }
    }

    /// Returns the root path of the project's workspace.
    pub fn root(&self) -> &Path {
        self.inner.root()
    }

    /// Returns the paths config.
    pub fn paths(&self) -> &ProjectPathsConfig {
        self.inner.paths()
    }

    /// Returns the path to the project's `foundry.toml` file.
    pub fn config(&self) -> PathBuf {
        self.root().join(Config::FILE_NAME)
    }

    /// Returns the path to the project's cache file.
    pub fn cache(&self) -> &PathBuf {
        &self.paths().cache
    }

    /// Returns the path to the project's artifacts directory.
    pub fn artifacts(&self) -> &PathBuf {
        &self.paths().artifacts
    }

    /// Removes this project's cache file.
    pub fn clear_cache(&self) {
        let _ = fs::remove_file(self.cache());
    }

    /// Removes this project's artifacts directory.
    pub fn clear_artifacts(&self) {
        let _ = fs::remove_dir_all(self.artifacts());
    }

    /// Writes the given config as toml to `foundry.toml`.
    pub fn write_config(&self, config: Config) {
        let file = self.config();
        pretty_err(&file, fs::write(&file, config.to_string_pretty().unwrap()));
    }

    /// Adds a source file to the project.
    pub fn add_source(&self, name: &str, contents: &str) -> SolcResult<PathBuf> {
        self.inner.add_source(name, Self::add_source_prelude(contents))
    }

    /// Adds a source file to the project. Prefer using `add_source` instead.
    pub fn add_raw_source(&self, name: &str, contents: &str) -> SolcResult<PathBuf> {
        self.inner.add_source(name, contents)
    }

    /// Adds a script file to the project.
    pub fn add_script(&self, name: &str, contents: &str) -> SolcResult<PathBuf> {
        self.inner.add_script(name, Self::add_source_prelude(contents))
    }

    /// Adds a test file to the project.
    pub fn add_test(&self, name: &str, contents: &str) -> SolcResult<PathBuf> {
        self.inner.add_test(name, Self::add_source_prelude(contents))
    }

    /// Adds a library file to the project.
    pub fn add_lib(&self, name: &str, contents: &str) -> SolcResult<PathBuf> {
        self.inner.add_lib(name, Self::add_source_prelude(contents))
    }

    fn add_source_prelude(s: &str) -> String {
        let mut s = s.to_string();
        if !s.contains("pragma solidity") {
            s = format!("pragma solidity ={SOLC_VERSION};\n{s}");
        }
        if !s.contains("// SPDX") {
            s = format!("// SPDX-License-Identifier: MIT OR Apache-2.0\n{s}");
        }
        s
    }

    /// Asserts that the `<root>/foundry.toml` file exists.
    #[track_caller]
    pub fn assert_config_exists(&self) {
        assert!(self.config().exists());
    }

    /// Asserts that the `<root>/cache/sol-files-cache.json` file exists.
    #[track_caller]
    pub fn assert_cache_exists(&self) {
        assert!(self.cache().exists());
    }

    /// Asserts that the `<root>/out` file exists.
    #[track_caller]
    pub fn assert_artifacts_dir_exists(&self) {
        assert!(self.paths().artifacts.exists());
    }

    /// Creates all project dirs and ensure they were created
    #[track_caller]
    pub fn assert_create_dirs_exists(&self) {
        self.paths().create_all().unwrap_or_else(|_| panic!("Failed to create project paths"));
        SolFilesCache::default().write(&self.paths().cache).expect("Failed to create cache");
        self.assert_all_paths_exist();
    }

    /// Ensures that the given layout exists
    #[track_caller]
    pub fn assert_style_paths_exist(&self, style: PathStyle) {
        let paths = style.paths(&self.paths().root).unwrap();
        config_paths_exist(&paths, self.inner.project().cached);
    }

    /// Copies the project's root directory to the given target
    #[track_caller]
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
        self.add_source("test.sol", s).unwrap()
    }

    /// Adds `console.sol` as a source under "console.sol"
    pub fn insert_console(&self) -> PathBuf {
        let s = include_str!("../../../testdata/logs/console.sol");
        self.add_source("console.sol", s).unwrap()
    }

    /// Asserts all project paths exist. These are:
    /// - sources
    /// - artifacts
    /// - libs
    /// - cache
    pub fn assert_all_paths_exist(&self) {
        let paths = self.paths();
        config_paths_exist(paths, self.inner.project().cached);
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
    pub fn forge_bin(&self) -> Command {
        let forge = self.exe_root.join(format!("../forge{}", env::consts::EXE_SUFFIX));
        let mut cmd = Command::new(forge);
        cmd.current_dir(self.inner.root());
        // disable color output for comparisons
        cmd.env("NO_COLOR", "1");
        cmd
    }

    /// Returns the path to the cast executable.
    pub fn cast_bin(&self) -> Command {
        let cast = self.exe_root.join(format!("../cast{}", env::consts::EXE_SUFFIX));
        let mut cmd = Command::new(cast);
        // disable color output for comparisons
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
        let c = lossy_string(&output.stdout);
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

/// A simple wrapper around a Command with some conveniences.
pub struct TestCommand {
    saved_cwd: PathBuf,
    /// The project used to launch this command.
    project: TestProject,
    /// The actual command we use to control the process.
    cmd: Command,
    // initial: Command,
    current_dir_lock: Option<parking_lot::lock_api::MutexGuard<'static, parking_lot::RawMutex, ()>>,
    stdin_fun: Option<Box<dyn FnOnce(ChildStdin)>>,
}

impl TestCommand {
    /// Returns a mutable reference to the underlying command.
    pub fn cmd(&mut self) -> &mut Command {
        &mut self.cmd
    }

    /// Replaces the underlying command.
    pub fn set_cmd(&mut self, cmd: Command) -> &mut TestCommand {
        self.cmd = cmd;
        self
    }

    /// Resets the command to the default `forge` command.
    pub fn forge_fuse(&mut self) -> &mut TestCommand {
        self.set_cmd(self.project.forge_bin())
    }

    /// Resets the command to the default `cast` command.
    pub fn cast_fuse(&mut self) -> &mut TestCommand {
        self.set_cmd(self.project.cast_bin())
    }

    /// Sets the current working directory.
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

    pub fn stdin(&mut self, fun: impl FnOnce(ChildStdin) + 'static) -> &mut TestCommand {
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
    #[track_caller]
    pub fn config(&mut self) -> Config {
        self.cmd.args(["config", "--json"]);
        let output = self.output();
        let c = lossy_string(&output.stdout);
        let config = serde_json::from_str(c.as_ref()).unwrap();
        self.forge_fuse();
        config
    }

    /// Runs `git init` inside the project's dir
    #[track_caller]
    pub fn git_init(&self) -> Output {
        let mut cmd = Command::new("git");
        cmd.arg("init").current_dir(self.project.root());
        let output = cmd.output().unwrap();
        self.ensure_success(&output).unwrap();
        output
    }

    /// Executes the command and returns the `(stdout, stderr)` of the output as lossy `String`s.
    ///
    /// Expects the command to be successful.
    #[track_caller]
    pub fn output_lossy(&mut self) -> (String, String) {
        let output = self.output();
        (lossy_string(&output.stdout), lossy_string(&output.stderr))
    }

    /// Executes the command and returns the `(stdout, stderr)` of the output as lossy `String`s.
    ///
    /// Does not expect the command to be successful.
    #[track_caller]
    pub fn unchecked_output_lossy(&mut self) -> (String, String) {
        let output = self.unchecked_output();
        (lossy_string(&output.stdout), lossy_string(&output.stderr))
    }

    /// Executes the command and returns the stderr as lossy `String`.
    ///
    /// **Note**: This function checks whether the command was successful.
    #[track_caller]
    pub fn stdout_lossy(&mut self) -> String {
        lossy_string(&self.output().stdout)
    }

    /// Executes the command and returns the stderr as lossy `String`.
    ///
    /// **Note**: This function does **not** check whether the command was successful.
    #[track_caller]
    pub fn stderr_lossy(&mut self) -> String {
        lossy_string(&self.unchecked_output().stderr)
    }

    /// Returns the output but does not expect that the command was successful
    #[track_caller]
    pub fn unchecked_output(&mut self) -> Output {
        self.execute()
    }

    /// Gets the output of a command. If the command failed, then this panics.
    #[track_caller]
    pub fn output(&mut self) -> Output {
        let output = self.execute();
        self.ensure_success(&output).unwrap();
        output
    }

    /// Runs the command and asserts that it resulted in success
    #[track_caller]
    pub fn assert_success(&mut self) {
        self.output();
    }

    /// Executes command, applies stdin function and returns output
    #[track_caller]
    pub fn execute(&mut self) -> Output {
        self.try_execute().unwrap()
    }

    #[track_caller]
    pub fn try_execute(&mut self) -> std::io::Result<Output> {
        eprintln!("executing {:?}", self.cmd);
        let mut child =
            self.cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::piped()).spawn()?;
        if let Some(fun) = self.stdin_fun.take() {
            fun(child.stdin.take().unwrap());
        }
        child.wait_with_output()
    }

    /// Executes command and expects an successful result
    #[track_caller]
    pub fn ensure_execute_success(&mut self) -> Result<Output> {
        let out = self.try_execute()?;
        self.ensure_success(&out)?;
        Ok(out)
    }

    /// Runs the command and prints its output
    /// You have to pass --nocapture to cargo test or the print won't be displayed.
    /// The full command would be: cargo test -- --nocapture
    #[track_caller]
    pub fn print_output(&mut self) {
        let output = self.execute();
        println!("stdout:\n{}", lossy_string(&output.stdout));
        println!("\nstderr:\n{}", lossy_string(&output.stderr));
    }

    /// Writes the content of the output to new fixture files
    #[track_caller]
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
        let out = self.execute();
        if out.status.success() {
            self.make_panic(&out, true);
        }
    }

    /// Runs the command and asserts that something was printed to stderr.
    #[track_caller]
    pub fn assert_non_empty_stderr(&mut self) {
        let out = self.execute();
        if out.status.success() || out.stderr.is_empty() {
            self.make_panic(&out, true);
        }
    }

    /// Runs the command and asserts that something was printed to stdout.
    #[track_caller]
    pub fn assert_non_empty_stdout(&mut self) {
        let out = self.execute();
        if !out.status.success() || out.stdout.is_empty() {
            self.make_panic(&out, false);
        }
    }

    /// Runs the command and asserts that nothing was printed to stdout.
    #[track_caller]
    pub fn assert_empty_stdout(&mut self) {
        let out = self.execute();
        if !out.status.success() || !out.stderr.is_empty() {
            self.make_panic(&out, true);
        }
    }

    #[track_caller]
    pub fn ensure_success(&self, out: &Output) -> Result<()> {
        if out.status.success() {
            Ok(())
        } else {
            Err(self.make_error(out, false))
        }
    }

    #[track_caller]
    fn make_panic(&self, out: &Output, expected_fail: bool) -> ! {
        panic!("{}", self.make_error_message(out, expected_fail))
    }

    #[track_caller]
    fn make_error(&self, out: &Output, expected_fail: bool) -> eyre::Report {
        eyre::eyre!("{}", self.make_error_message(out, expected_fail))
    }

    fn make_error_message(&self, out: &Output, expected_fail: bool) -> String {
        let msg = if expected_fail {
            "expected failure but command succeeded!"
        } else {
            "command failed but expected success!"
        };
        format!(
            "\
--- {:?} ---
{msg}

status: {}

paths:
{}

stdout:
{}

stderr:
{}",
            self.cmd,
            out.status,
            self.project.inner.paths(),
            lossy_string(&out.stdout),
            lossy_string(&out.stderr),
        )
    }
}

/// Extension trait for [`Output`].
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
    let re = &[
        r"\r",
        // solc version
        r"\s*Solc(?: version)? \d+.\d+.\d+\s*",
        r" files with \d+.\d+.\d+",
        // solc runs
        r"runs: \d+, μ: \d+, ~: \d+",
        // elapsed time
        "finished in (.*)?s\n",
        // file paths
        r"-->(.*).sol",
        r"Location(.|\n)*\.rs(.|\n)*Backtrace",
        // other
        r"Transaction hash: 0x[0-9A-Fa-f]{64}",
    ];
    Regex::new(&format!("({})", re.join("|"))).unwrap()
});

impl OutputExt for Output {
    #[track_caller]
    fn stdout_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self {
        let expected = fs::read_to_string(expected_path).unwrap();
        let expected = IGNORE_IN_FIXTURES.replace_all(&expected, "").replace('\\', "/");
        let stdout = lossy_string(&self.stdout);
        let out = IGNORE_IN_FIXTURES.replace_all(&stdout, "").replace('\\', "/");

        pretty_assertions::assert_eq!(expected, out);

        self
    }

    #[track_caller]
    fn stderr_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self {
        let expected = fs::read_to_string(expected_path).unwrap();
        let expected = IGNORE_IN_FIXTURES.replace_all(&expected, "").replace('\\', "/");
        let stderr = lossy_string(&self.stderr);
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

fn lossy_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
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
            r"
Location:
   [35mcli/src/compile.rs[0m:[35m151[0m

Backtrace omitted.
        "
        ));
    }
}
