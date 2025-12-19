use crate::{init_tracing, rpc::rpc_endpoints};
use eyre::{Result, WrapErr};
use foundry_compilers::{
    ArtifactOutput, ConfigurableArtifacts, PathStyle, ProjectPathsConfig,
    artifacts::Contract,
    cache::CompilerCache,
    compilers::multi::MultiCompiler,
    project_util::{TempProject, copy_dir},
    solc::SolcSettings,
};
use foundry_config::Config;
use parking_lot::Mutex;
use regex::Regex;
use snapbox::{Data, IntoData, assert_data_eq, cmd::OutputAssert};
use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::util::{SOLC_VERSION, pretty_err};

static CURRENT_DIR_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Global test identifier.
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Clones a remote repository into the specified directory. Panics if the command fails.
pub fn clone_remote(repo_url: &str, target_dir: &str, recursive: bool) {
    let mut cmd = Command::new("git");
    cmd.args(["clone"]);
    if recursive {
        cmd.args(["--recursive", "--shallow-submodules"]);
    } else {
        cmd.args(["--depth=1", "--no-checkout", "--filter=blob:none", "--no-recurse-submodules"]);
    }
    cmd.args([repo_url, target_dir]);
    test_debug!("{cmd:?}");
    let status = cmd.status().unwrap();
    if !status.success() {
        panic!("git clone failed: {status}");
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
#[derive(Clone, Debug)]
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

/// Same as `setup_forge_remote` but not panicking
pub fn try_setup_forge_remote(
    config: impl Into<RemoteProject>,
) -> Result<(TestProject, TestCommand)> {
    let config = config.into();
    let mut tmp = TempProject::checkout(&config.id).wrap_err("failed to checkout project")?;
    tmp.project_mut().paths = config.path_style.paths(tmp.root())?;

    let prj = TestProject::with_project(tmp);
    if config.run_build {
        let mut cmd = prj.forge_command();
        cmd.arg("build").assert_success();
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
pub struct TestProject<
    T: ArtifactOutput<CompilerContract = Contract> + Default = ConfigurableArtifacts,
> {
    /// The directory in which this test executable is running.
    exe_root: PathBuf,
    /// The project in which the test should run.
    pub(crate) inner: Arc<TempProject<MultiCompiler, T>>,
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
        let exe_root = canonicalize(this.parent().expect("executable's directory"));
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

    /// Removes the project's cache and artifacts directory.
    pub fn clear(&self) {
        self.clear_cache();
        self.clear_artifacts();
    }

    /// Removes this project's cache file.
    pub fn clear_cache(&self) {
        let _ = fs::remove_file(self.cache());
    }

    /// Removes this project's artifacts directory.
    pub fn clear_artifacts(&self) {
        let _ = fs::remove_dir_all(self.artifacts());
    }

    /// Removes the entire cache directory (including fuzz, invariant, and test-failures caches).
    pub fn clear_cache_dir(&self) {
        let _ = fs::remove_dir_all(self.root().join("cache"));
    }

    /// Updates the project's config with the given function.
    pub fn update_config(&self, f: impl FnOnce(&mut Config)) {
        self._update_config(Box::new(f));
    }

    fn _update_config(&self, f: Box<dyn FnOnce(&mut Config) + '_>) {
        let mut config = self
            .config()
            .exists()
            .then_some(())
            .and_then(|()| Config::load_with_root(self.root()).ok())
            .unwrap_or_default();
        config.remappings.clear();
        f(&mut config);
        self.write_config(config);
    }

    /// Writes the given config as toml to `foundry.toml`.
    #[doc(hidden)] // Prefer `update_config`.
    pub fn write_config(&self, config: Config) {
        let file = self.config();
        pretty_err(&file, fs::write(&file, config.to_string_pretty().unwrap()));
    }

    /// Writes [`rpc_endpoints`] to the project's config.
    pub fn add_rpc_endpoints(&self) {
        self.update_config(|config| {
            config.rpc_endpoints = rpc_endpoints();
        });
    }

    /// Adds a source file to the project.
    pub fn add_source(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_source(name, Self::add_source_prelude(contents)).unwrap()
    }

    /// Adds a source file to the project. Prefer using `add_source` instead.
    pub fn add_raw_source(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_source(name, contents).unwrap()
    }

    /// Adds a script file to the project.
    pub fn add_script(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_script(name, Self::add_source_prelude(contents)).unwrap()
    }

    /// Adds a script file to the project. Prefer using `add_script` instead.
    pub fn add_raw_script(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_script(name, contents).unwrap()
    }

    /// Adds a test file to the project.
    pub fn add_test(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_test(name, Self::add_source_prelude(contents)).unwrap()
    }

    /// Adds a test file to the project. Prefer using `add_test` instead.
    pub fn add_raw_test(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_test(name, contents).unwrap()
    }

    /// Adds a library file to the project.
    pub fn add_lib(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_lib(name, Self::add_source_prelude(contents)).unwrap()
    }

    /// Adds a library file to the project. Prefer using `add_lib` instead.
    pub fn add_raw_lib(&self, name: &str, contents: &str) -> PathBuf {
        self.inner.add_lib(name, contents).unwrap()
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
        CompilerCache::<SolcSettings>::default()
            .write(&self.paths().cache)
            .expect("Failed to create cache");
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
        self.add_source("test.sol", include_str!("../../../testdata/utils/DSTest.sol"))
    }

    /// Adds custom test utils under the "test/utils" directory.
    pub fn insert_utils(&self) {
        self.add_test("utils/DSTest.sol", include_str!("../../../testdata/utils/DSTest.sol"));
        self.add_test("utils/Test.sol", include_str!("../../../testdata/utils/Test.sol"));
        self.add_test("utils/Vm.sol", include_str!("../../../testdata/utils/Vm.sol"));
        self.add_test("utils/console.sol", include_str!("../../../testdata/utils/console.sol"));
    }

    /// Adds `console.sol` as a source under "console.sol"
    pub fn insert_console(&self) -> PathBuf {
        let s = include_str!("../../../testdata/utils/console.sol");
        self.add_source("console.sol", s)
    }

    /// Adds `Vm.sol` as a source under "Vm.sol"
    pub fn insert_vm(&self) -> PathBuf {
        let s = include_str!("../../../testdata/utils/Vm.sol");
        self.add_source("Vm.sol", s)
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
            stdin: None,
            redact_output: true,
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
            stdin: None,
            redact_output: true,
        }
    }

    /// Returns the path to the forge executable.
    pub fn forge_bin(&self) -> Command {
        let mut cmd = Command::new(self.forge_path());
        cmd.current_dir(self.inner.root());
        // Disable color output for comparisons; can be overridden with `--color always`.
        cmd.env("NO_COLOR", "1");
        cmd
    }

    pub(crate) fn forge_path(&self) -> PathBuf {
        canonicalize(self.exe_root.join(format!("../forge{}", env::consts::EXE_SUFFIX)))
    }

    /// Returns the path to the cast executable.
    pub fn cast_bin(&self) -> Command {
        let cast = canonicalize(self.exe_root.join(format!("../cast{}", env::consts::EXE_SUFFIX)));
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

    /// Initializes the default contracts (Counter.sol, Counter.t.sol, Counter.s.sol).
    ///
    /// This is useful for tests that need the default contracts created by `forge init`.
    /// Most tests should not need this method, as the default behavior is to create an empty
    /// project.
    pub fn initialize_default_contracts(&self) {
        self.add_raw_source(
            "Counter.sol",
            include_str!("../../forge/assets/solidity/CounterTemplate.sol"),
        );
        self.add_raw_test(
            "Counter.t.sol",
            include_str!("../../forge/assets/solidity/CounterTemplate.t.sol"),
        );
        self.add_raw_script(
            "Counter.s.sol",
            include_str!("../../forge/assets/solidity/CounterTemplate.s.sol"),
        );
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

/// A simple wrapper around a Command with some conveniences.
pub struct TestCommand {
    saved_cwd: PathBuf,
    /// The project used to launch this command.
    project: TestProject,
    /// The actual command we use to control the process.
    cmd: Command,
    // initial: Command,
    current_dir_lock: Option<parking_lot::MutexGuard<'static, ()>>,
    stdin: Option<Vec<u8>>,
    /// If true, command output is redacted.
    redact_output: bool,
}

impl TestCommand {
    /// Returns a mutable reference to the underlying command.
    pub fn cmd(&mut self) -> &mut Command {
        &mut self.cmd
    }

    /// Replaces the underlying command.
    pub fn set_cmd(&mut self, cmd: Command) -> &mut Self {
        self.cmd = cmd;
        self
    }

    /// Resets the command to the default `forge` command.
    pub fn forge_fuse(&mut self) -> &mut Self {
        self.set_cmd(self.project.forge_bin())
    }

    /// Resets the command to the default `cast` command.
    pub fn cast_fuse(&mut self) -> &mut Self {
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
    pub fn arg<A: AsRef<OsStr>>(&mut self, arg: A) -> &mut Self {
        self.cmd.arg(arg);
        self
    }

    /// Add any number of arguments to the command.
    pub fn args<I, A>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = A>,
        A: AsRef<OsStr>,
    {
        self.cmd.args(args);
        self
    }

    /// Set the stdin bytes for the next command.
    pub fn stdin(&mut self, stdin: impl Into<Vec<u8>>) -> &mut Self {
        self.stdin = Some(stdin.into());
        self
    }

    /// Convenience function to add `--root project.root()` argument
    pub fn root_arg(&mut self) -> &mut Self {
        let root = self.project.root().to_path_buf();
        self.arg("--root").arg(root)
    }

    /// Set the environment variable `k` to value `v` for the command.
    pub fn env(&mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) {
        self.cmd.env(k, v);
    }

    /// Set the environment variable `k` to value `v` for the command.
    pub fn envs<I, K, V>(&mut self, envs: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.cmd.envs(envs);
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
    pub fn current_dir<P: AsRef<Path>>(&mut self, dir: P) -> &mut Self {
        self.cmd.current_dir(dir);
        self
    }

    /// Returns the `Config` as spit out by `forge config`
    #[track_caller]
    pub fn config(&mut self) -> Config {
        self.cmd.args(["config", "--json"]);
        let output = self.assert().success().get_output().stdout_lossy();
        self.forge_fuse();
        serde_json::from_str(output.as_ref()).unwrap()
    }

    /// Runs `git init` inside the project's dir
    #[track_caller]
    pub fn git_init(&self) {
        let mut cmd = Command::new("git");
        cmd.arg("init").current_dir(self.project.root());
        let output = OutputAssert::new(cmd.output().unwrap());
        output.success();
    }

    /// Runs `git submodule status` inside the project's dir
    #[track_caller]
    pub fn git_submodule_status(&self) -> Output {
        let mut cmd = Command::new("git");
        cmd.arg("submodule").arg("status").current_dir(self.project.root());
        cmd.output().unwrap()
    }

    /// Runs `git add .` inside the project's dir
    #[track_caller]
    pub fn git_add(&self) {
        let mut cmd = Command::new("git");
        cmd.current_dir(self.project.root());
        cmd.arg("add").arg(".");
        let output = OutputAssert::new(cmd.output().unwrap());
        output.success();
    }

    /// Runs `git commit .` inside the project's dir
    #[track_caller]
    pub fn git_commit(&self, msg: &str) {
        let mut cmd = Command::new("git");
        cmd.current_dir(self.project.root());
        cmd.arg("commit").arg("-m").arg(msg);
        let output = OutputAssert::new(cmd.output().unwrap());
        output.success();
    }

    /// Runs the command, returning a [`snapbox`] object to assert the command output.
    #[track_caller]
    pub fn assert_with(&mut self, f: &[RegexRedaction]) -> OutputAssert {
        let assert = OutputAssert::new(self.execute());
        if self.redact_output {
            let mut redactions = test_redactions();
            insert_redactions(f, &mut redactions);
            return assert.with_assert(
                snapbox::Assert::new()
                    .action_env(snapbox::assert::DEFAULT_ACTION_ENV)
                    .redact_with(redactions),
            );
        }
        assert
    }

    /// Runs the command, returning a [`snapbox`] object to assert the command output.
    #[track_caller]
    pub fn assert(&mut self) -> OutputAssert {
        self.assert_with(&[])
    }

    /// Runs the command and asserts that it resulted in success.
    #[track_caller]
    pub fn assert_success(&mut self) -> OutputAssert {
        self.assert().success()
    }

    /// Runs the command and asserts that it resulted in success, with expected JSON data.
    #[track_caller]
    pub fn assert_json_stdout(&mut self, expected: impl IntoData) {
        let expected = expected.is(snapbox::data::DataFormat::Json).unordered();
        let stdout = self.assert_success().get_output().stdout.clone();
        let actual = stdout.into_data().is(snapbox::data::DataFormat::Json).unordered();
        assert_data_eq!(actual, expected);
    }

    /// Runs the command and asserts that it resulted in the expected outcome and JSON data.
    #[track_caller]
    pub fn assert_json_stderr(&mut self, success: bool, expected: impl IntoData) {
        let expected = expected.is(snapbox::data::DataFormat::Json).unordered();
        let stderr = if success { self.assert_success() } else { self.assert_failure() }
            .get_output()
            .stderr
            .clone();
        let actual = stderr.into_data().is(snapbox::data::DataFormat::Json).unordered();
        assert_data_eq!(actual, expected);
    }

    /// Runs the command and asserts that it **succeeded** nothing was printed to stdout.
    #[track_caller]
    pub fn assert_empty_stdout(&mut self) {
        self.assert_success().stdout_eq(Data::new());
    }

    /// Runs the command and asserts that it failed.
    #[track_caller]
    pub fn assert_failure(&mut self) -> OutputAssert {
        self.assert().failure()
    }

    /// Runs the command and asserts that the exit code is `expected`.
    #[track_caller]
    pub fn assert_code(&mut self, expected: i32) -> OutputAssert {
        self.assert().code(expected)
    }

    /// Runs the command and asserts that it **failed** nothing was printed to stderr.
    #[track_caller]
    pub fn assert_empty_stderr(&mut self) {
        self.assert_failure().stderr_eq(Data::new());
    }

    /// Runs the command with a temporary file argument and asserts that the contents of the file
    /// match the given data.
    #[track_caller]
    pub fn assert_file(&mut self, data: impl IntoData) {
        self.assert_file_with(|this, path| _ = this.arg(path).assert_success(), data);
    }

    /// Creates a temporary file, passes it to `f`, then asserts that the contents of the file match
    /// the given data.
    #[track_caller]
    pub fn assert_file_with(&mut self, f: impl FnOnce(&mut Self, &Path), data: impl IntoData) {
        let file = tempfile::NamedTempFile::new().expect("couldn't create temporary file");
        f(self, file.path());
        assert_data_eq!(Data::read_from(file.path(), None), data);
    }

    /// Does not apply [`snapbox`] redactions to the command output.
    pub fn with_no_redact(&mut self) -> &mut Self {
        self.redact_output = false;
        self
    }

    /// Executes command, applies stdin function and returns output
    #[track_caller]
    pub fn execute(&mut self) -> Output {
        self.try_execute().unwrap()
    }

    #[track_caller]
    pub fn try_execute(&mut self) -> std::io::Result<Output> {
        test_debug!("executing {:?}", self.cmd);
        let mut child =
            self.cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::piped()).spawn()?;
        if let Some(bytes) = self.stdin.take() {
            child.stdin.take().unwrap().write_all(&bytes)?;
        }
        let output = child.wait_with_output()?;
        test_debug!("exited with {}", output.status);
        test_trace!("\n--- stdout ---\n{}\n--- /stdout ---", output.stdout_lossy());
        test_trace!("\n--- stderr ---\n{}\n--- /stderr ---", output.stderr_lossy());
        Ok(output)
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

fn test_redactions() -> snapbox::Redactions {
    static REDACTIONS: LazyLock<snapbox::Redactions> = LazyLock::new(|| {
        make_redactions(&[
            ("[SOLC_VERSION]", r"Solc( version)? \d+.\d+.\d+"),
            ("[ELAPSED]", r"(finished )?in \d+(\.\d+)?\w?s( \(.*?s CPU time\))?"),
            ("[GAS]", r"[Gg]as( used)?: \d+"),
            ("[GAS_COST]", r"[Gg]as cost\s*\(\d+\)"),
            ("[GAS_LIMIT]", r"[Gg]as limit\s*\(\d+\)"),
            ("[AVG_GAS]", r"μ: \d+, ~: \d+"),
            ("[FILE]", r"-->.*\.sol"),
            ("[FILE]", r"Location(.|\n)*\.rs(.|\n)*Backtrace"),
            ("[COMPILING_FILES]", r"Compiling \d+ files?"),
            ("[TX_HASH]", r"Transaction hash: 0x[0-9A-Fa-f]{64}"),
            ("[ADDRESS]", r"Address: +0x[0-9A-Fa-f]{40}"),
            ("[PUBLIC_KEY]", r"Public key: +0x[0-9A-Fa-f]{128}"),
            ("[PRIVATE_KEY]", r"Private key: +0x[0-9A-Fa-f]{64}"),
            ("[UPDATING_DEPENDENCIES]", r"Updating dependencies in .*"),
            ("[SAVED_TRANSACTIONS]", r"Transactions saved to: .*\.json"),
            ("[SAVED_SENSITIVE_VALUES]", r"Sensitive values saved to: .*\.json"),
            ("[ESTIMATED_GAS_PRICE]", r"Estimated gas price:\s*(\d+(\.\d+)?)\s*gwei"),
            ("[ESTIMATED_TOTAL_GAS_USED]", r"Estimated total gas used for script: \d+"),
            (
                "[ESTIMATED_AMOUNT_REQUIRED]",
                r"Estimated amount required:\s*(\d+(\.\d+)?)\s*[A-Z]{3}",
            ),
        ])
    });
    REDACTIONS.clone()
}

/// A tuple of a placeholder and a regex replacement string.
pub type RegexRedaction = (&'static str, &'static str);

/// Creates a [`snapbox`] redactions object from a list of regex redactions.
fn make_redactions(redactions: &[RegexRedaction]) -> snapbox::Redactions {
    let mut r = snapbox::Redactions::new();
    insert_redactions(redactions, &mut r);
    r
}

fn insert_redactions(redactions: &[RegexRedaction], r: &mut snapbox::Redactions) {
    for &(placeholder, re) in redactions {
        r.insert(placeholder, Regex::new(re).expect(re)).expect(re);
    }
}

/// Extension trait for [`Output`].
pub trait OutputExt {
    /// Returns the stdout as lossy string
    fn stdout_lossy(&self) -> String;

    /// Returns the stderr as lossy string
    fn stderr_lossy(&self) -> String;
}

impl OutputExt for Output {
    fn stdout_lossy(&self) -> String {
        lossy_string(&self.stdout)
    }

    fn stderr_lossy(&self) -> String {
        lossy_string(&self.stderr)
    }
}

pub fn lossy_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn canonicalize(path: impl AsRef<Path>) -> PathBuf {
    foundry_common::fs::canonicalize_path(path.as_ref())
        .unwrap_or_else(|_| path.as_ref().to_path_buf())
}
