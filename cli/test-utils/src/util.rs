use atty::{self, Stream};
use ethers_solc::{
    cache::SolFilesCache,
    project_util::{copy_dir, TempProject},
    ArtifactOutput, ConfigurableArtifacts, PathStyle, ProjectPathsConfig, Solc,
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
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::{self, Command},
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
pub static IS_TTY: Lazy<bool> = Lazy::new(|| atty::is(Stream::Stdout));

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
        v0_8_10.join().unwrap().unwrap();
        v0_8_13.join().unwrap().unwrap();

        *is_preinstalled = true;
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
        }
    }

    /// Creates a new command that is set to use the cast executable for this project
    pub fn cast_command(&self) -> TestCommand {
        let mut cmd = self.cast_bin();
        cmd.current_dir(&self.inner.root());
        let _lock = CURRENT_DIR_LOCK.lock();
        TestCommand {
            project: self.clone(),
            cmd,
            current_dir_lock: None,
            saved_cwd: pretty_err("<current dir>", std::env::current_dir()),
        }
    }

    /// Returns the path to the forge executable.
    pub fn forge_bin(&self) -> process::Command {
        let forge = self.root.join(format!("../forge{}", env::consts::EXE_SUFFIX));
        let mut cmd = process::Command::new(forge);
        cmd.current_dir(self.inner.root());
        cmd
    }

    /// Returns the path to the cast executable.
    pub fn cast_bin(&self) -> process::Command {
        let cast = self.root.join(format!("../cast{}", env::consts::EXE_SUFFIX));
        process::Command::new(cast)
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
        Err(err) => panic!("{}: {:?}", path.as_ref().display(), err),
    }
}

pub fn read_string(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    pretty_err(path, std::fs::read_to_string(path))
}

/// A simple wrapper around a process::Command with some conveniences.
#[derive(Debug)]
pub struct TestCommand {
    saved_cwd: PathBuf,
    /// The project used to launch this command.
    project: TestProject,
    /// The actual command we use to control the process.
    cmd: Command,
    // initial: Command,
    current_dir_lock: Option<parking_lot::lock_api::MutexGuard<'static, parking_lot::RawMutex, ()>>,
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
        match stdout.parse() {
            Ok(t) => t,
            Err(err) => {
                panic!("could not convert from string: {:?}\n\n{}", err, stdout);
            }
        }
    }

    /// Returns the `stderr` of the output as `String`.
    pub fn stderr_lossy(&mut self) -> String {
        let output = self.cmd.output().unwrap();
        String::from_utf8_lossy(&output.stderr).to_string()
    }

    /// Returns the `stdout` of the output as `String`.
    pub fn stdout_lossy(&mut self) -> String {
        String::from_utf8_lossy(&self.output().stdout).to_string()
    }

    /// Returns the output but does not expect that the command was successful
    pub fn unchecked_output(&mut self) -> process::Output {
        self.cmd.output().unwrap()
    }

    /// Gets the output of a command. If the command failed, then this panics.
    pub fn output(&mut self) -> process::Output {
        let output = self.cmd.output().unwrap();
        self.expect_success(output)
    }

    /// Runs the command and prints its output
    pub fn print_output(&mut self) {
        let output = self.cmd.output().unwrap();
        println!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        println!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    /// Writes the content of the output to new fixture files
    pub fn write_fixtures(&mut self, name: impl AsRef<Path>) {
        let name = name.as_ref();
        if let Some(parent) = name.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let output = self.cmd.output().unwrap();
        fs::write(format!("{}.stdout", name.display()), &output.stdout).unwrap();
        fs::write(format!("{}.stderr", name.display()), &output.stderr).unwrap();
    }

    /// Runs the command and asserts that it resulted in an error exit code.
    pub fn assert_err(&mut self) {
        let o = self.cmd.output().unwrap();
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
    pub fn assert_non_empty_stderr(&mut self) {
        let o = self.cmd.output().unwrap();
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
    pub fn assert_non_empty_stdout(&mut self) {
        let o = self.cmd.output().unwrap();
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
    pub fn assert_empty_stdout(&mut self) {
        let o = self.cmd.output().unwrap();
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

    fn expect_success(&self, out: process::Output) -> process::Output {
        if !out.status.success() {
            let suggest = if out.stderr.is_empty() {
                "\n\nDid your forge command end up with no output?".to_string()
            } else {
                "".to_string()
            };
            panic!(
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
        out
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
    Regex::new(r"(finished in (.*)?s|-->(.*).sol|Location(.*?)\.rs(.*)?Backtrace|installing solc version(.*?)\n|Successfully installed solc(.*?)\n)").unwrap()
});

impl OutputExt for process::Output {
    #[track_caller]
    fn stdout_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self {
        let expected = fs::read_to_string(expected_path).unwrap();
        let expected = IGNORE_IN_FIXTURES.replace_all(&expected, "");
        let stdout = String::from_utf8_lossy(&self.stdout);
        let out = IGNORE_IN_FIXTURES.replace_all(&stdout, "");

        pretty_assertions::assert_eq!(expected, out);

        self
    }

    #[track_caller]
    fn stderr_matches_path(&self, expected_path: impl AsRef<Path>) -> &Self {
        let expected = fs::read_to_string(expected_path).unwrap();
        let expected = IGNORE_IN_FIXTURES.replace_all(&expected, "");
        let stderr = String::from_utf8_lossy(&self.stderr);
        let out = IGNORE_IN_FIXTURES.replace_all(&stderr, "");

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
            path.with_extension(format!("tty.{}", ext))
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
}
