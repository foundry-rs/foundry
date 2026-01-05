use crate::prj::{TestCommand, TestProject, clone_remote, setup_forge};
use foundry_compilers::PathStyle;
use std::process::Command;

/// External test builder
#[derive(Clone, Debug)]
#[must_use = "ExtTester does nothing unless you `run` it"]
pub struct ExtTester {
    pub org: &'static str,
    pub name: &'static str,
    pub rev: &'static str,
    pub style: PathStyle,
    pub fork_block: Option<u64>,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
    pub install_commands: Vec<Vec<String>>,
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
            args: vec![],
            envs: vec![],
            install_commands: vec![],
            verbosity: "-vvv".to_string(),
        }
    }

    /// Sets the path style.
    pub fn style(mut self, style: PathStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the fork block.
    pub fn fork_block(mut self, fork_block: u64) -> Self {
        self.fork_block = Some(fork_block);
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

    pub fn setup_forge_prj(&self, recursive: bool) -> (TestProject, TestCommand) {
        let (prj, mut test_cmd) = setup_forge(self.name, self.style.clone());

        // Export vyper and forge in test command - workaround for snekmate venom tests.
        if let Some(vyper) = &prj.inner.project().compiler.vyper {
            let vyper_dir = vyper.path.parent().expect("vyper path should have a parent");
            let forge_bin = prj.forge_path();
            let forge_dir = forge_bin.parent().expect("forge path should have a parent");

            let existing_path = std::env::var_os("PATH").unwrap_or_default();
            let mut new_paths = vec![vyper_dir.to_path_buf(), forge_dir.to_path_buf()];
            new_paths.extend(std::env::split_paths(&existing_path));

            let joined_path = std::env::join_paths(new_paths).expect("failed to join PATH");
            test_cmd.env("PATH", joined_path);
        }

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
            if !output.status.success() {
                panic!("git log failed: {output:?}");
            }
            let stdout = String::from_utf8(output.stdout).unwrap();
            let commit = stdout.lines().next().unwrap().split_whitespace().nth(1).unwrap();
            panic!("pin to latest commit: {commit}");
        } else {
            let mut git = Command::new("git");
            git.current_dir(root).args(["checkout", self.rev]);
            test_debug!("$ {git:?}");
            let status = git.status().unwrap();
            if !status.success() {
                panic!("git checkout failed: {status}");
            }
        }

        (prj, test_cmd)
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
        test_cmd.args(["--fuzz-runs=32", "--ffi", &self.verbosity]);

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
