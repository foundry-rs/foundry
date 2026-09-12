use crate::prj::{TestCommand, TestProject, clone_remote, setup_forge};
use foundry_compilers::PathStyle;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// External test builder
#[derive(Clone, Debug)]
#[must_use = "ExtTester does nothing unless you `run` it"]
pub struct ExtTester {
    pub org: &'static str,
    pub name: &'static str,
    pub rev: &'static str,
    pub style: PathStyle,
    pub fork_block: Option<u64>,
    pub fuzz_runs: u32,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
    pub install_commands: Vec<Vec<String>>,
    pub python_packages: Vec<String>,
    pub verbosity: String,
}

impl ExtTester {
    /// Creates a new external test builder.
    pub fn new(org: &'static str, name: &'static str, rev: &'static str) -> Self {
        Self {
            org,
            name,
            rev,
            style: PathStyle::Dapptools,
            fork_block: None,
            fuzz_runs: 32,
            args: vec![],
            envs: vec![],
            install_commands: vec![],
            python_packages: vec![],
            verbosity: "-vvv".to_string(),
        }
    }

    /// Sets the path style.
    pub const fn style(mut self, style: PathStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the fork block.
    pub const fn fork_block(mut self, fork_block: u64) -> Self {
        self.fork_block = Some(fork_block);
        self
    }

    /// Sets the number of fuzz runs.
    pub const fn fuzz_runs(mut self, fuzz_runs: u32) -> Self {
        self.fuzz_runs = fuzz_runs;
        self
    }

    /// Adds an argument to the forge command.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Adds multiple arguments to the forge command.
    pub fn args<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets the verbosity
    pub fn verbosity(mut self, verbosity: usize) -> Self {
        self.verbosity = format!("-{}", "v".repeat(verbosity));
        self
    }

    /// Adds an environment variable to the forge command.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Adds multiple environment variables to the forge command.
    pub fn envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.envs.extend(envs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Adds a command to run after the project is cloned.
    ///
    /// Note that the command is run in the project's root directory, and it won't fail the test if
    /// it fails.
    pub fn install_command(mut self, command: &[&str]) -> Self {
        self.install_commands.push(command.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Adds a Python package to install in a fixture-local virtual environment.
    pub fn python_package(mut self, package: impl Into<String>) -> Self {
        self.python_packages.push(package.into());
        self
    }

    pub fn setup_forge_prj(&self, recursive: bool) -> (TestProject, TestCommand) {
        let (prj, mut test_cmd) = setup_forge(self.name, self.style.clone());

        // Wipe the default structure.
        prj.wipe();

        // Clone the external repository.
        let repo_url = format!("https://github.com/{}/{}.git", self.org, self.name);
        let root = prj.root().to_str().unwrap();
        clone_remote(&repo_url, root, recursive);

        // Checkout the revision.
        if self.rev.is_empty() {
            let mut git = Command::new("git");
            git.current_dir(root).args(["log", "-n", "1"]);
            test_debug!("$ {git:?}");
            let output = git.output().unwrap();
            assert!(output.status.success(), "git log failed: {output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let commit = stdout.lines().next().unwrap().split_whitespace().nth(1).unwrap();
            panic!("pin to latest commit: {commit}");
        }
        checkout_revision(root, self.rev, recursive);

        // Export fixture-local Python packages, vyper, and forge in the test command.
        let mut new_paths = Vec::new();
        if let Some(python_bin_dir) = self.install_python_packages(root) {
            new_paths.push(python_bin_dir);
        }
        if let Some(vyper) = &prj.inner.project().compiler.vyper {
            let vyper_dir = vyper.path.parent().expect("vyper path should have a parent");
            new_paths.push(vyper_dir.to_path_buf());
        }
        let forge_bin = prj.foundry_bin_path("forge");
        let forge_dir = forge_bin.parent().expect("forge path should have a parent");
        new_paths.push(forge_dir.to_path_buf());
        let existing_path = std::env::var_os("PATH").unwrap_or_default();
        new_paths.extend(std::env::split_paths(&existing_path));

        let joined_path = std::env::join_paths(new_paths).expect("failed to join PATH");
        test_cmd.env("PATH", joined_path);

        (prj, test_cmd)
    }

    fn install_python_packages(&self, root: &str) -> Option<PathBuf> {
        if self.python_packages.is_empty() {
            return None;
        }

        let venv = Path::new(root).join(".foundry-ext-venv");
        let mut venv_cmd = Command::new("python3");
        venv_cmd.args(["-m", "venv"]).arg(&venv);
        test_debug!("cd {root}; {venv_cmd:?}");
        let status = venv_cmd.current_dir(root).status().expect("failed to create Python venv");
        assert!(status.success(), "python venv creation failed: {status}");

        let bin_dir = venv.join(if cfg!(windows) { "Scripts" } else { "bin" });
        let pip = bin_dir.join(if cfg!(windows) { "pip.exe" } else { "pip" });
        for package in &self.python_packages {
            let mut pip_cmd = Command::new(&pip);
            pip_cmd.args(["install", "--disable-pip-version-check"]).arg(package).current_dir(root);
            test_debug!("cd {root}; {pip_cmd:?}");
            let status = pip_cmd.status().expect("failed to install Python package");
            assert!(status.success(), "Python package install failed: {status}");
        }

        Some(bin_dir)
    }

    pub fn run_install_commands(&self, root: &str) {
        for install_command in &self.install_commands {
            let mut install_cmd = Command::new(&install_command[0]);
            install_cmd.args(&install_command[1..]).current_dir(root);
            test_debug!("cd {root}; {install_cmd:?}");
            match install_cmd.status() {
                Ok(s) => {
                    test_debug!("\n\n{install_cmd:?}: {s}");
                    if s.success() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("\n\n{install_cmd:?}: {e}");
                }
            }
        }
    }

    /// Runs the test.
    pub fn run(&self) {
        let (prj, mut test_cmd) = self.setup_forge_prj(true);

        // Run installation command.
        self.run_install_commands(prj.root().to_str().unwrap());

        // Run the tests.
        test_cmd.arg("test");
        test_cmd.args(&self.args);
        test_cmd.args([
            format!("--fuzz-runs={}", self.fuzz_runs),
            "--ffi".to_string(),
            self.verbosity.clone(),
        ]);

        test_cmd.envs(self.envs.iter().map(|(k, v)| (k, v)));
        if let Some(fork_block) = self.fork_block {
            test_cmd.env("FOUNDRY_ETH_RPC_URL", crate::rpc::next_http_archive_rpc_url());
            test_cmd.env("FOUNDRY_FORK_BLOCK_NUMBER", fork_block.to_string());
        }
        test_cmd.env("FOUNDRY_INVARIANT_DEPTH", "15");
        test_cmd.env("FOUNDRY_ALLOW_INTERNAL_EXPECT_REVERT", "true");

        test_cmd.assert_success();
    }
}

/// Checks out a fixture revision and restores its pinned submodule revisions.
fn checkout_revision(root: &str, rev: &str, recursive: bool) {
    checkout_revision_inner(root, rev, recursive, None);
}

fn checkout_revision_inner(root: &str, rev: &str, recursive: bool, allowed_protocol: Option<&str>) {
    let mut git = Command::new("git");
    if let Some(protocol) = allowed_protocol {
        git.env("GIT_ALLOW_PROTOCOL", protocol);
    }
    git.current_dir(root).args(["checkout", rev]);
    test_debug!("$ {git:?}");
    let status = git.status().unwrap();
    assert!(status.success(), "git checkout failed: {status}");

    if recursive {
        // The clone initialized submodules from the default branch, not the pinned revision.
        for args in [
            &["-c", "submodule.recurse=false", "submodule", "sync"][..],
            &["-c", "submodule.recurse=false", "submodule", "update", "--init", "--checkout"][..],
            &[
                "submodule",
                "foreach",
                "--recursive",
                "git -c submodule.recurse=false submodule sync && git -c submodule.recurse=false \
                 submodule update --init --checkout",
            ][..],
        ] {
            let mut git = Command::new("git");
            if let Some(protocol) = allowed_protocol {
                git.env("GIT_ALLOW_PROTOCOL", protocol);
            }
            git.current_dir(root).args(args);
            test_debug!("$ {git:?}");
            let status = git.status().unwrap();
            assert!(status.success(), "git {args:?} failed: {status}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(root)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn checkout_revision_restores_pinned_submodule() {
        let temp = tempfile::tempdir().unwrap();
        let dependency = temp.path().join("dependency");
        let fixture = temp.path().join("fixture");
        fs::create_dir(&dependency).unwrap();
        fs::create_dir(&fixture).unwrap();
        git(&dependency, &["init"]);
        fs::write(dependency.join("draft.sol"), "old interface").unwrap();
        git(&dependency, &["add", "."]);
        git(&dependency, &["commit", "-m", "old interface"]);
        let pinned_dependency = git(&dependency, &["rev-parse", "HEAD"]);

        git(&fixture, &["init"]);
        git(
            &fixture,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                dependency.to_str().unwrap(),
                "lib/dependency",
            ],
        );
        git(&fixture, &["commit", "-am", "pin old dependency"]);
        let pinned_fixture = git(&fixture, &["rev-parse", "HEAD"]);
        git(&fixture, &["config", "submodule.recurse", "false"]);

        let submodule = fixture.join("lib/dependency");
        git(&submodule, &["mv", "draft.sol", "interface.sol"]);
        git(&submodule, &["commit", "-m", "rename interface"]);
        git(&fixture, &["commit", "-am", "update dependency"]);
        let updated_fixture = git(&fixture, &["rev-parse", "HEAD"]);

        // Checking out only the parent leaves the dependency at the newer revision.
        checkout_revision(fixture.to_str().unwrap(), &pinned_fixture, false);
        assert!(!submodule.join("draft.sol").exists());
        assert!(submodule.join("interface.sol").exists());

        checkout_revision(fixture.to_str().unwrap(), &updated_fixture, false);
        git(&fixture, &["config", "submodule.lib/dependency.update", "merge"]);
        checkout_revision(fixture.to_str().unwrap(), &pinned_fixture, true);
        assert_eq!(git(&submodule, &["rev-parse", "HEAD"]), pinned_dependency);
        assert!(submodule.join("draft.sol").exists());
        assert!(!submodule.join("interface.sol").exists());
        assert!(git(&fixture, &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn checkout_revision_syncs_nested_submodule_after_parent_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let old_nested = temp.path().join("old-nested");
        let new_nested = temp.path().join("new-nested");
        let dependency = temp.path().join("dependency");
        let fixture = temp.path().join("fixture");
        let checkout = temp.path().join("checkout");

        for (repository, file) in [(&old_nested, "old.sol"), (&new_nested, "new.sol")] {
            fs::create_dir(repository).unwrap();
            git(repository, &["init"]);
            fs::write(repository.join(file), file).unwrap();
            git(repository, &["add", "."]);
            git(repository, &["commit", "-m", file]);
        }
        let pinned_nested = git(&old_nested, &["rev-parse", "HEAD"]);

        fs::create_dir(&dependency).unwrap();
        git(&dependency, &["init"]);
        git(
            &dependency,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                old_nested.to_str().unwrap(),
                "lib/nested",
            ],
        );
        git(&dependency, &["commit", "-am", "pin old nested dependency"]);
        let pinned_dependency = git(&dependency, &["rev-parse", "HEAD"]);

        fs::create_dir(&fixture).unwrap();
        git(&fixture, &["init"]);
        git(
            &fixture,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                dependency.to_str().unwrap(),
                "lib/dependency",
            ],
        );
        git(&fixture, &["commit", "-am", "pin old dependency"]);
        let pinned_fixture = git(&fixture, &["rev-parse", "HEAD"]);

        git(
            &dependency,
            &[
                "config",
                "--file",
                ".gitmodules",
                "submodule.lib/nested.url",
                new_nested.to_str().unwrap(),
            ],
        );
        git(&dependency, &["submodule", "sync"]);
        let nested = dependency.join("lib/nested");
        let updated_nested = git(&new_nested, &["rev-parse", "HEAD"]);
        git(&nested, &["fetch", "origin"]);
        git(&nested, &["checkout", &updated_nested]);
        git(&dependency, &["add", "."]);
        git(&dependency, &["commit", "-m", "use new nested dependency"]);
        let updated_dependency = git(&dependency, &["rev-parse", "HEAD"]);
        let fixture_dependency = fixture.join("lib/dependency");
        git(&fixture_dependency, &["fetch", "origin"]);
        git(&fixture_dependency, &["checkout", &updated_dependency]);
        git(&fixture, &["add", "lib/dependency"]);
        git(&fixture, &["commit", "-m", "update dependency"]);

        git(
            temp.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "clone",
                "--recursive",
                fixture.to_str().unwrap(),
                checkout.to_str().unwrap(),
            ],
        );
        git(&checkout, &["config", "submodule.recurse", "false"]);

        checkout_revision_inner(checkout.to_str().unwrap(), &pinned_fixture, true, Some("file"));

        let checked_out_dependency = checkout.join("lib/dependency");
        let checked_out_nested = checked_out_dependency.join("lib/nested");
        assert_eq!(git(&checked_out_dependency, &["rev-parse", "HEAD"]), pinned_dependency);
        assert_eq!(git(&checked_out_nested, &["rev-parse", "HEAD"]), pinned_nested);
        assert_eq!(
            git(&checked_out_nested, &["remote", "get-url", "origin"]),
            old_nested.to_str().unwrap()
        );
        assert!(checked_out_nested.join("old.sol").exists());
        assert!(!checked_out_nested.join("new.sol").exists());
        assert!(git(&checkout, &["status", "--porcelain"]).is_empty());
    }
}
