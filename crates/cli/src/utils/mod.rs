use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, U256, map::HashMap};
use alloy_provider::{Network, Provider, RootProvider, network::AnyNetwork};
use eyre::{ContextCompat, Result};
use foundry_common::{provider::ProviderBuilder, shell};
use foundry_config::{Chain, Config};
use itertools::Itertools;
use path_slash::PathExt;
use regex::Regex;
use serde::de::DeserializeOwned;
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    str::FromStr,
    sync::{LazyLock, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tracing_subscriber::{EnvFilter, prelude::*, reload};

mod cmd;
pub use cmd::*;

mod suggestions;
pub use suggestions::*;

mod abi;
pub use abi::*;

mod allocator;
pub use allocator::*;

mod tempo;
pub use tempo::*;

// reexport all `foundry_config::utils`
#[doc(hidden)]
pub use foundry_config::utils::*;

/// Deterministic fuzzer seed used for gas snapshots and coverage reports.
///
/// The keccak256 hash of "foundry rulez"
pub const STATIC_FUZZ_SEED: [u8; 32] = [
    0x01, 0x00, 0xfa, 0x69, 0xa5, 0xf1, 0x71, 0x0a, 0x95, 0xcd, 0xef, 0x94, 0x88, 0x9b, 0x02, 0x84,
    0x5d, 0x64, 0x0b, 0x19, 0xad, 0xf0, 0xe3, 0x57, 0xb8, 0xd4, 0xbe, 0x7d, 0x49, 0xee, 0x70, 0xe6,
];

/// Regex used to parse `.gitmodules` file and capture the submodule path and branch.
pub static SUBMODULE_BRANCH_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\[submodule "([^"]+)"\](?:[^\[]*?branch = ([^\s]+))"#).unwrap());
/// Regex used to parse `git submodule status` output.
pub static SUBMODULE_STATUS_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\s+-]?([a-f0-9]+)\s+([^\s]+)(?:\s+\([^)]+\))?$").unwrap());

/// Handle to reload the tracing `EnvFilter` at runtime.
static FILTER_RELOAD_HANDLE: OnceLock<reload::Handle<EnvFilter, tracing_subscriber::Registry>> =
    OnceLock::new();

/// Useful extensions to [`std::path::Path`].
pub trait FoundryPathExt {
    /// Returns true if the [`Path`] ends with `.t.sol`
    fn is_sol_test(&self) -> bool;

    /// Returns true if the  [`Path`] has a `sol` extension
    fn is_sol(&self) -> bool;

    /// Returns true if the  [`Path`] has a `yul` extension
    fn is_yul(&self) -> bool;
}

impl<T: AsRef<Path>> FoundryPathExt for T {
    fn is_sol_test(&self) -> bool {
        self.as_ref()
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.ends_with(".t.sol"))
            .unwrap_or_default()
    }

    fn is_sol(&self) -> bool {
        self.as_ref().extension() == Some(std::ffi::OsStr::new("sol"))
    }

    fn is_yul(&self) -> bool {
        self.as_ref().extension() == Some(std::ffi::OsStr::new("yul"))
    }
}

/// Initializes a tracing Subscriber for logging.
///
/// The `EnvFilter` is wrapped in a [`reload::Layer`] so it can be reconfigured at runtime via
/// [`update_tracing_filter`].
pub fn subscriber() {
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter());
    let registry = tracing_subscriber::Registry::default().with(filter_layer);
    #[cfg(feature = "tracy")]
    let registry = registry.with(tracing_tracy::TracyLayer::default());
    registry.with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr)).init();
    let _ = FILTER_RELOAD_HANDLE.set(reload_handle);
}

/// Replaces the active tracing `EnvFilter` at runtime.
///
/// `directives` is parsed as an [`EnvFilter`] (e.g. `"info"`, `"debug,hyper=off"`).
/// This is a no-op if [`subscriber`] has not been called yet.
pub fn update_tracing_filter(directives: &str) {
    let Some(handle) = FILTER_RELOAD_HANDLE.get() else {
        return;
    };
    let Ok(new_filter) = directives.parse::<EnvFilter>() else {
        return;
    };
    let _ = handle.reload(new_filter);
}

fn env_filter() -> EnvFilter {
    const DEFAULT_DIRECTIVES: &[&str] = &include!("./default_directives.txt");
    let mut filter = EnvFilter::from_default_env();
    for &directive in DEFAULT_DIRECTIVES {
        filter = filter.add_directive(directive.parse().unwrap());
    }
    filter
}

/// Returns a [`RootProvider`] instantiated using [Config]'s RPC settings.
pub fn get_provider(config: &Config) -> Result<RootProvider<AnyNetwork>> {
    get_provider_builder(config)?.build()
}

/// Returns a [ProviderBuilder] instantiated using [Config] values.
///
/// Defaults to `http://localhost:8545` and `Mainnet`.
pub fn get_provider_builder(config: &Config) -> Result<ProviderBuilder> {
    ProviderBuilder::from_config(config)
}

pub async fn get_chain<N, P>(chain: Option<Chain>, provider: P) -> Result<Chain>
where
    N: Network,
    P: Provider<N>,
{
    match chain {
        Some(chain) => Ok(chain),
        None => Ok(Chain::from_id(provider.get_chain_id().await?)),
    }
}

/// Parses an ether value from a string.
///
/// The amount can be tagged with a unit, e.g. "1ether".
///
/// If the string represents an untagged amount (e.g. "100") then
/// it is interpreted as wei.
pub fn parse_ether_value(value: &str) -> Result<U256> {
    Ok(if value.starts_with("0x") || value.starts_with("0X") {
        U256::from_str(value)?
    } else {
        alloy_dyn_abi::DynSolType::coerce_str(&alloy_dyn_abi::DynSolType::Uint(256), value)?
            .as_uint()
            .wrap_err("Could not parse ether value from string")?
            .0
    })
}

/// Parses a `T` from a string using [`serde_json::from_str`].
pub fn parse_json<T: DeserializeOwned>(value: &str) -> serde_json::Result<T> {
    serde_json::from_str(value)
}

/// Parses a `Duration` from a &str
pub fn parse_delay(delay: &str) -> Result<Duration> {
    let delay = if delay.ends_with("ms") {
        let d: u64 = delay.trim_end_matches("ms").parse()?;
        Duration::from_millis(d)
    } else {
        let d: f64 = delay.parse()?;
        let delay = (d * 1000.0).round();
        if delay.is_infinite() || delay.is_nan() || delay.is_sign_negative() {
            eyre::bail!("delay must be finite and non-negative");
        }

        Duration::from_millis(delay as u64)
    };
    Ok(delay)
}

/// Returns the current time as a [`Duration`] since the Unix epoch.
pub fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("time went backwards")
}

/// Common setup for all CLI tools. Does not include [tracing subscriber](subscriber).
pub fn common_setup() {
    install_crypto_provider();
    crate::handler::install();
    load_dotenv();
    enable_paint();
}

/// Loads a dotenv file, from the cwd and the project root, ignoring potential failure.
///
/// We could use `warn!` here, but that would imply that the dotenv file can't configure
/// the logging behavior of Foundry.
///
/// Similarly, we could just use `eprintln!`, but colors are off limits otherwise dotenv is implied
/// to not be able to configure the colors. It would also mess up the JSON output.
pub fn load_dotenv() {
    let load = |p: &Path| {
        dotenvy::from_path(p.join(".env")).ok();
    };

    // we only want the .env file of the cwd and project root
    // `find_project_root` calls `current_dir` internally so both paths are either both `Ok` or
    // both `Err`
    if let (Ok(cwd), Ok(prj_root)) = (std::env::current_dir(), find_project_root(None)) {
        load(&prj_root);
        if cwd != prj_root {
            // prj root and cwd can be identical
            load(&cwd);
        }
    };
}

/// Sets the default [`yansi`] color output condition.
pub fn enable_paint() {
    let enable = yansi::Condition::os_support() && yansi::Condition::tty_and_color_live();
    yansi::whenever(yansi::Condition::cached(enable));
}

/// This force installs the default crypto provider.
///
/// This is necessary in case there are more than one available backends enabled in rustls (ring,
/// aws-lc-rs).
///
/// This should be called high in the main fn.
///
/// See also:
///   <https://github.com/snapview/tokio-tungstenite/issues/353#issuecomment-2455100010>
///   <https://github.com/awslabs/aws-sdk-rust/discussions/1257>
pub fn install_crypto_provider() {
    // https://github.com/snapview/tokio-tungstenite/issues/353
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default rustls crypto provider");
}

/// Fetches the ABI of a contract from Etherscan.
pub async fn fetch_abi_from_etherscan(
    address: Address,
    config: &foundry_config::Config,
) -> Result<Vec<(JsonAbi, String)>> {
    let chain = config.chain.unwrap_or_default();
    let client = config
        .get_etherscan_config_with_chain(Some(chain))?
        .ok_or_else(|| eyre::eyre!("No Etherscan API key configured for chain {chain}"))?
        .into_client_with_no_proxy(config.eth_rpc_no_proxy)?;
    let source = client.contract_source_code(address).await?;
    source.items.into_iter().map(|item| Ok((item.abi()?, item.contract_name))).collect()
}

/// Useful extensions to [`std::process::Command`].
pub trait CommandUtils {
    /// Returns the command's output if execution is successful, otherwise, throws an error.
    fn exec(&mut self) -> Result<Output>;

    /// Returns the command's stdout if execution is successful, otherwise, throws an error.
    fn get_stdout_lossy(&mut self) -> Result<String>;
}

impl CommandUtils for Command {
    #[track_caller]
    fn exec(&mut self) -> Result<Output> {
        trace!(command=?self, "executing");

        let output = self.output()?;

        trace!(code=?output.status.code(), ?output);

        if output.status.success() {
            Ok(output)
        } else {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stdout = stdout.trim();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            let msg = if stdout.is_empty() {
                stderr.to_string()
            } else if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("stdout:\n{stdout}\n\nstderr:\n{stderr}")
            };

            let mut name = self.get_program().to_string_lossy();
            if let Some(arg) = self.get_args().next() {
                let arg = arg.to_string_lossy();
                if !arg.starts_with('-') {
                    let name = name.to_mut();
                    name.push(' ');
                    name.push_str(&arg);
                }
            }

            let mut err = match output.status.code() {
                Some(code) => format!("{name} exited with code {code}"),
                None => format!("{name} terminated by a signal"),
            };
            if !msg.is_empty() {
                err.push(':');
                err.push(if msg.lines().count() == 1 { ' ' } else { '\n' });
                err.push_str(&msg);
            }
            Err(eyre::eyre!(err))
        }
    }

    #[track_caller]
    fn get_stdout_lossy(&mut self) -> Result<String> {
        let output = self.exec()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().into())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Git<'a> {
    pub root: &'a Path,
    pub quiet: bool,
    pub shallow: bool,
}

impl<'a> Git<'a> {
    pub fn new(root: &'a Path) -> Self {
        Self { root, quiet: shell::is_quiet(), shallow: false }
    }

    pub fn from_config(config: &'a Config) -> Self {
        Self::new(config.root.as_path())
    }

    pub fn root_of(relative_to: &Path) -> Result<PathBuf> {
        let output = Self::cmd_no_root()
            .current_dir(relative_to)
            .args(["rev-parse", "--show-toplevel"])
            .get_stdout_lossy()?;
        Ok(PathBuf::from(output))
    }

    pub fn clone_with_branch(
        shallow: bool,
        from: impl AsRef<OsStr>,
        branch: impl AsRef<OsStr>,
        to: Option<impl AsRef<OsStr>>,
    ) -> Result<()> {
        Self::cmd_no_root()
            .stderr(Stdio::inherit())
            .args(["clone", "--recurse-submodules"])
            .args(shallow.then_some("--depth=1"))
            .args(shallow.then_some("--shallow-submodules"))
            .arg("-b")
            .arg(branch)
            .arg(from)
            .args(to)
            .exec()
            .map(drop)
    }

    pub fn clone(
        shallow: bool,
        from: impl AsRef<OsStr>,
        to: Option<impl AsRef<OsStr>>,
    ) -> Result<()> {
        Self::cmd_no_root()
            .stderr(Stdio::inherit())
            .args(["clone", "--recurse-submodules"])
            .args(shallow.then_some("--depth=1"))
            .args(shallow.then_some("--shallow-submodules"))
            .arg(from)
            .args(to)
            .exec()
            .map(drop)
    }

    pub fn fetch(
        self,
        shallow: bool,
        remote: impl AsRef<OsStr>,
        branch: Option<impl AsRef<OsStr>>,
    ) -> Result<()> {
        self.cmd()
            .stderr(Stdio::inherit())
            .arg("fetch")
            .args(shallow.then_some("--no-tags"))
            .args(shallow.then_some("--depth=1"))
            .arg(remote)
            .args(branch)
            .exec()
            .map(drop)
    }

    pub const fn root(self, root: &Path) -> Git<'_> {
        Git { root, ..self }
    }

    pub const fn quiet(self, quiet: bool) -> Self {
        Self { quiet, ..self }
    }

    /// True to perform shallow clones
    pub const fn shallow(self, shallow: bool) -> Self {
        Self { shallow, ..self }
    }

    pub fn checkout(self, recursive: bool, tag: impl AsRef<OsStr>) -> Result<()> {
        self.cmd()
            .arg("checkout")
            .args(recursive.then_some("--recurse-submodules"))
            .arg(tag)
            .exec()
            .map(drop)
    }

    /// Returns the current HEAD commit hash of the current branch.
    pub fn head(self) -> Result<String> {
        self.cmd().args(["rev-parse", "HEAD"]).get_stdout_lossy()
    }

    pub fn checkout_at(self, tag: impl AsRef<OsStr>, at: &Path) -> Result<()> {
        self.cmd_at(at).arg("checkout").arg(tag).exec().map(drop)
    }

    pub fn init(self) -> Result<()> {
        self.cmd().arg("init").exec().map(drop)
    }

    pub fn current_rev_branch(self, at: &Path) -> Result<(String, String)> {
        let rev = self.cmd_at(at).args(["rev-parse", "HEAD"]).get_stdout_lossy()?;
        let branch =
            self.cmd_at(at).args(["rev-parse", "--abbrev-ref", "HEAD"]).get_stdout_lossy()?;
        Ok((rev, branch))
    }

    #[expect(clippy::should_implement_trait)] // this is not std::ops::Add clippy
    pub fn add<I, S>(self, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd().arg("add").args(paths).exec().map(drop)
    }

    pub fn add_literal(self, path: &Path) -> Result<()> {
        self.cmd().args(["--literal-pathspecs", "add", "--"]).arg(path).exec().map(drop)
    }

    pub fn reset(self, hard: bool, tree: impl AsRef<OsStr>) -> Result<()> {
        self.cmd().arg("reset").args(hard.then_some("--hard")).arg(tree).exec().map(drop)
    }

    pub fn commit_tree(
        self,
        tree: impl AsRef<OsStr>,
        msg: Option<impl AsRef<OsStr>>,
    ) -> Result<String> {
        self.cmd()
            .arg("commit-tree")
            .arg(tree)
            .args(msg.as_ref().is_some().then_some("-m"))
            .args(msg)
            .get_stdout_lossy()
    }

    pub fn rm<I, S>(self, force: bool, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd().arg("rm").args(force.then_some("--force")).arg("--").args(paths).exec().map(drop)
    }

    pub fn remove_index_path(self, path: &Path) -> Result<()> {
        self.cmd()
            .args(["--literal-pathspecs", "rm", "--cached", "--force", "--"])
            .arg(path)
            .exec()
            .map(drop)
    }

    pub fn commit(self, msg: &str) -> Result<()> {
        let output = self
            .cmd()
            .args(["commit", "-m", msg])
            .args(cfg!(any(test, debug_assertions)).then_some("--no-gpg-sign"))
            .output()?;
        if !output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            // ignore "nothing to commit" error
            let msg = "nothing to commit, working tree clean";
            if !(stdout.contains(msg) || stderr.contains(msg)) {
                return Err(eyre::eyre!(
                    "failed to commit (code={:?}, stdout={:?}, stderr={:?})",
                    output.status.code(),
                    stdout.trim(),
                    stderr.trim()
                ));
            }
        }
        Ok(())
    }

    pub fn is_in_repo(self) -> std::io::Result<bool> {
        self.cmd().args(["rev-parse", "--is-inside-work-tree"]).status().map(|s| s.success())
    }

    pub fn is_repo_root(self) -> Result<bool> {
        self.cmd().args(["rev-parse", "--show-cdup"]).get_stdout_lossy().map(|s| s.is_empty())
    }

    pub fn is_clean(self) -> Result<bool> {
        self.cmd().args(["status", "--porcelain"]).exec().map(|out| out.stdout.is_empty())
    }

    pub fn is_path_clean(self, path: &Path) -> Result<bool> {
        self.cmd()
            .args(["--literal-pathspecs", "status", "--porcelain", "--"])
            .arg(path)
            .exec()
            .map(|out| out.stdout.is_empty())
    }

    /// Returns whether a path has no unstaged changes relative to the index.
    pub fn is_path_worktree_clean(self, path: &Path) -> Result<bool> {
        let output = self.cmd().args(["diff", "--quiet", "--"]).arg(path).output()?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(eyre::eyre!(
                "failed to inspect path: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }

    pub fn has_branch(self, branch: impl AsRef<OsStr>, at: &Path) -> Result<bool> {
        self.cmd_at(at)
            .args(["branch", "--list", "--no-color"])
            .arg(branch)
            .get_stdout_lossy()
            .map(|stdout| !stdout.is_empty())
    }

    pub fn has_tag(self, tag: impl AsRef<OsStr>, at: &Path) -> Result<bool> {
        self.cmd_at(at)
            .args(["tag", "--list"])
            .arg(tag)
            .get_stdout_lossy()
            .map(|stdout| !stdout.is_empty())
    }

    pub fn has_rev(self, rev: impl AsRef<OsStr>, at: &Path) -> Result<bool> {
        self.cmd_at(at)
            .args(["cat-file", "-t"])
            .arg(rev)
            .get_stdout_lossy()
            .map(|stdout| &stdout == "commit")
    }

    pub fn get_rev(self, tag_or_branch: impl AsRef<OsStr>, at: &Path) -> Result<String> {
        self.cmd_at(at).args(["rev-list", "-n", "1"]).arg(tag_or_branch).get_stdout_lossy()
    }

    pub fn ensure_clean(self) -> Result<()> {
        if self.is_clean()? {
            Ok(())
        } else {
            Err(eyre::eyre!(
                "\
The target directory is a part of or on its own an already initialized git repository,
and it requires clean working and staging areas, including no untracked files.

Check the current git repository's status with `git status`.
Then, you can track files with `git add ...` and then commit them with `git commit`,
ignore them in the `.gitignore` file."
            ))
        }
    }

    pub fn commit_hash(self, short: bool, revision: &str) -> Result<String> {
        self.cmd()
            .arg("rev-parse")
            .args(short.then_some("--short"))
            .arg(revision)
            .get_stdout_lossy()
    }

    pub fn tag(self) -> Result<String> {
        self.cmd().arg("tag").get_stdout_lossy()
    }

    /// Returns the tag the commit first appeared in.
    ///
    /// E.g Take rev = `abc1234`. This commit can be found in multiple releases (tags).
    /// Consider releases: `v0.1.0`, `v0.2.0`, `v0.3.0` in chronological order, `rev` first appeared
    /// in `v0.2.0`.
    ///
    /// Hence, `tag_for_commit("abc1234")` will return `v0.2.0`.
    pub fn tag_for_commit(self, rev: &str, at: &Path) -> Result<Option<String>> {
        self.cmd_at(at)
            .args(["tag", "--contains"])
            .arg(rev)
            .get_stdout_lossy()
            .map(|stdout| stdout.lines().next().map(str::to_string))
    }

    /// Returns a list of tuples of submodule paths and their respective branches.
    ///
    /// This function reads the `.gitmodules` file and returns the paths of all submodules that have
    /// a branch. The paths are relative to the Git::root_of(git.root) and not lib/ directory.
    ///
    /// `at` is the dir in which the `.gitmodules` file is located, this is the git root.
    /// `lib` is name of the directory where the submodules are located.
    pub fn read_submodules_with_branch(
        self,
        at: &Path,
        lib: &OsStr,
    ) -> Result<HashMap<PathBuf, String>> {
        // Read the .gitmodules file
        let gitmodules = foundry_common::fs::read_to_string(at.join(".gitmodules"))?;

        let paths = SUBMODULE_BRANCH_REGEX
            .captures_iter(&gitmodules)
            .map(|cap| {
                let path_str = cap.get(1).unwrap().as_str();
                let path = PathBuf::from_str(path_str).unwrap();
                trace!(path = %path.display(), "unstripped path");

                // Keep only the components that come after the lib directory.
                // This needs to be done because the lockfile uses paths relative foundry project
                // root whereas .gitmodules use paths relative to the git root which may not be the
                // project root. e.g monorepo.
                // Hence, if path is lib/solady, then `lib/solady` is kept. if path is
                // packages/contract-bedrock/lib/solady, then `lib/solady` is kept.
                let lib_pos = path.components().find_position(|c| c.as_os_str() == lib);
                let path = path
                    .components()
                    .skip(lib_pos.map(|(i, _)| i).unwrap_or(0))
                    .collect::<PathBuf>();

                let branch = cap.get(2).unwrap().as_str().to_string();
                (path, branch)
            })
            .collect::<HashMap<_, _>>();

        Ok(paths)
    }

    pub fn has_missing_dependencies<I, S>(self, paths: I) -> Result<bool>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let paths = paths.into_iter().map(|path| path.as_ref().to_owned()).collect::<Vec<_>>();
        if self.submodules_initialized(&paths).unwrap_or(false) {
            return Ok(false);
        }

        self.cmd()
            .args(["submodule", "status"])
            .args(&paths)
            .get_stdout_lossy()
            .map(|stdout| stdout.lines().any(|line| line.starts_with('-')))
    }

    /// Returns true if all submodules matching `paths` have initialized worktrees.
    fn submodules_initialized(self, paths: &[OsString]) -> Result<bool> {
        let Some(root) = self.root.ancestors().find(|root| root.join(".git").exists()) else {
            return Ok(false);
        };
        if paths.iter().any(|path| {
            Path::new(path)
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        }) {
            return Ok(false);
        }
        let relative_root = self.root.strip_prefix(root).unwrap_or_else(|_| Path::new(""));
        let gitmodules = root.join(".gitmodules");
        if !gitmodules.is_file() {
            return Ok(false);
        }

        let output = Command::new("git")
            .args(["config", "--null", "--file"])
            .arg(gitmodules)
            .args(["--get-regexp", r"^submodule\..*\.path$"])
            .get_stdout_lossy()?;

        for entry in output.split_terminator('\0') {
            let (_, path) = entry
                .split_once('\n')
                .ok_or_else(|| eyre::eyre!("invalid submodule path config"))?;
            let path = Path::new(path);
            let matches = paths.is_empty()
                || paths.iter().any(|prefix| {
                    let prefix = Path::new(prefix);
                    if prefix.is_absolute() {
                        prefix.strip_prefix(root).is_ok_and(|prefix| path.starts_with(prefix))
                    } else {
                        path.starts_with(relative_root.join(prefix))
                    }
                });
            if matches && !root.join(path).join(".git").exists() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Returns true if the given path has submodules by checking `git submodule status`
    pub fn has_submodules<I, S>(self, paths: I) -> Result<bool>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd()
            .args(["submodule", "status"])
            .args(paths)
            .get_stdout_lossy()
            .map(|stdout| stdout.trim().lines().next().is_some())
    }

    pub fn submodule_add(
        self,
        force: bool,
        url: impl AsRef<OsStr>,
        path: impl AsRef<OsStr>,
    ) -> Result<()> {
        self.cmd()
            .stderr(self.stderr())
            .args(["submodule", "add"])
            .args(self.shallow.then_some("--depth=1"))
            .args(force.then_some("--force"))
            .arg(url)
            .arg(path)
            .exec()
            .map(drop)
    }

    pub fn submodule_update<I, S>(
        self,
        force: bool,
        remote: bool,
        no_fetch: bool,
        recursive: bool,
        paths: I,
    ) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.cmd()
            .stderr(self.stderr())
            .args(["--literal-pathspecs", "submodule", "update", "--progress", "--init"])
            .args(self.shallow.then_some("--depth=1"))
            .args(force.then_some("--force"))
            .args(remote.then_some("--remote"))
            .args(no_fetch.then_some("--no-fetch"))
            .args(recursive.then_some("--recursive"))
            .arg("--")
            .args(paths)
            .exec()
            .map(drop)
    }

    pub fn submodule_foreach(self, recursive: bool, cmd: impl AsRef<OsStr>) -> Result<()> {
        self.cmd()
            .stderr(self.stderr())
            .args(["submodule", "foreach"])
            .args(recursive.then_some("--recursive"))
            .arg(cmd)
            .exec()
            .map(drop)
    }

    /// If the status is prefix with `-`, the submodule is not initialized.
    ///
    /// Ref: <https://git-scm.com/docs/git-submodule#Documentation/git-submodule.txt-status--cached--recursive--ltpathgt82308203>
    pub fn submodules_uninitialized(self) -> Result<bool> {
        // keep behavior consistent with `has_missing_dependencies`, but avoid duplicating the
        // "submodule status has '-' prefix" logic.
        self.has_missing_dependencies(std::iter::empty::<&OsStr>())
    }

    /// Initializes the git submodules.
    pub fn submodule_init(self) -> Result<()> {
        self.cmd().stderr(self.stderr()).args(["submodule", "init"]).exec().map(drop)
    }

    pub fn submodule_deinit(self, force: bool, path: &Path) -> Result<()> {
        self.cmd()
            .stderr(self.stderr())
            .args(["--literal-pathspecs", "submodule", "deinit"])
            .args(force.then_some("--force"))
            .arg("--")
            .arg(path)
            .exec()
            .map(drop)
    }

    pub fn submodules(&self) -> Result<Submodules> {
        self.cmd().args(["submodule", "status"]).get_stdout_lossy().map(|stdout| stdout.parse())?
    }

    pub fn submodule_sync(self) -> Result<()> {
        self.cmd().stderr(self.stderr()).args(["submodule", "sync"]).exec().map(drop)
    }

    /// Get the URL of a submodule from git config
    pub fn submodule_url(self, path: &Path) -> Result<Option<String>> {
        self.cmd()
            .args(["config", "--get", &format!("submodule.{}.url", path.to_slash_lossy())])
            .get_stdout_lossy()
            .map(|url| Some(url.trim().to_string()))
    }

    /// Returns whether the default section name conflicts and the mapping for an exact path.
    pub fn submodule_mapping_for_path(
        self,
        path: &Path,
    ) -> Result<(bool, Option<SubmoduleMapping>)> {
        let gitmodules = self.root.join(".gitmodules");
        if !gitmodules.exists() {
            return Ok((false, None));
        }

        let output = self
            .cmd()
            .args(["config", "--null", "--file"])
            .arg(gitmodules)
            .args(["--get-regexp", r"^submodule\..*"])
            .output()?;
        match output.status.code() {
            Some(0) => {
                let expected_path = path.to_slash_lossy();
                let mut sections = HashMap::<String, (Option<String>, Option<String>)>::default();
                for entry in
                    output.stdout.split(|byte| *byte == 0).filter(|entry| !entry.is_empty())
                {
                    let Some(separator) = entry.iter().position(|byte| *byte == b'\n') else {
                        return Err(eyre::eyre!("invalid submodule mapping entry"));
                    };
                    let key = std::str::from_utf8(&entry[..separator])?;
                    let value = std::str::from_utf8(&entry[separator + 1..])?;
                    let Some(key) = key.strip_prefix("submodule.") else { continue };
                    let Some((name, field)) = key.rsplit_once('.') else { continue };
                    let section = sections.entry(name.to_string()).or_default();
                    match field {
                        "path" if section.0.is_none() => section.0 = Some(value.to_string()),
                        "url" if section.1.is_none() => section.1 = Some(value.to_string()),
                        _ => {}
                    }
                }

                let default_name_exists = sections.contains_key(expected_path.as_ref());
                let mut mappings = sections
                    .into_iter()
                    .filter(|(_, (mapped_path, _))| mapped_path.as_deref() == Some(&expected_path))
                    .map(|(name, (_, url))| SubmoduleMapping { name, url });
                let mapping = mappings.next();
                let multiple_mappings = mappings.next().is_some();
                let name_conflict = multiple_mappings
                    || default_name_exists
                        && mapping.as_ref().is_none_or(|mapping| mapping.name != expected_path);
                Ok((name_conflict, (!multiple_mappings).then_some(mapping).flatten()))
            }
            Some(1) => Ok((false, None)),
            _ => Err(eyre::eyre!(
                "failed to inspect .gitmodules: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }

    /// Returns whether the index contains a submodule at the given path.
    pub fn is_gitlink(self, path: &Path) -> Result<bool> {
        self.cmd().args(["ls-files", "--stage", "-z", "--"]).arg(path).exec().map(|output| {
            let expected_path = path.to_slash_lossy();
            output.stdout.split(|byte| *byte == 0).any(|entry| {
                entry.starts_with(b"160000 ")
                    && entry
                        .iter()
                        .position(|byte| *byte == b'\t')
                        .and_then(|separator| entry.get(separator + 1..))
                        == Some(expected_path.as_bytes())
            })
        })
    }

    /// Returns whether the index contains any entry at or below the given path.
    pub fn has_index_entries(self, path: &Path) -> Result<bool> {
        self.cmd()
            .args(["--literal-pathspecs", "ls-files", "--stage", "-z", "--"])
            .arg(path)
            .exec()
            .map(|output| !output.stdout.is_empty())
    }

    /// Returns whether the index contains the exact path.
    pub fn is_tracked(self, path: &Path) -> Result<bool> {
        self.cmd().args(["ls-files", "-z", "--"]).arg(path).exec().map(|output| {
            let expected_path = path.to_slash_lossy();
            output.stdout.split(|byte| *byte == 0).any(|entry| entry == expected_path.as_bytes())
        })
    }

    /// Returns all local config values for the given submodule.
    pub fn submodule_config(self, path: &Path) -> Result<Vec<(String, String)>> {
        let pattern = format!(r"^submodule\.{}\.", regex::escape(&path.to_slash_lossy()));
        let output =
            self.cmd().args(["config", "--local", "--null", "--get-regexp", &pattern]).output()?;
        match output.status.code() {
            Some(0) => output
                .stdout
                .split(|byte| *byte == 0)
                .filter(|entry| !entry.is_empty())
                .map(|entry| {
                    let Some(separator) = entry.iter().position(|byte| *byte == b'\n') else {
                        return Err(eyre::eyre!("invalid submodule config entry"));
                    };
                    let (key, value) = entry.split_at(separator);
                    let value = &value[1..];
                    Ok((String::from_utf8(key.to_vec())?, String::from_utf8(value.to_vec())?))
                })
                .collect(),
            Some(1) => Ok(Vec::new()),
            _ => Err(eyre::eyre!(
                "failed to inspect submodule config: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }

    /// Replaces all local config values for the given submodule.
    pub fn restore_submodule_config(self, path: &Path, config: &[(String, String)]) -> Result<()> {
        if !self.submodule_config(path)?.is_empty() {
            let section = format!("submodule.{}", path.to_slash_lossy());
            self.cmd().args(["config", "--local", "--remove-section", &section]).exec()?;
        }
        for (key, value) in config {
            self.cmd().args(["config", "--local", "--add", key, value]).exec()?;
        }
        Ok(())
    }

    /// Returns the absolute path to the repository's Git directory.
    pub fn absolute_git_dir(self) -> Result<PathBuf> {
        self.cmd().args(["rev-parse", "--absolute-git-dir"]).get_stdout_lossy().map(PathBuf::from)
    }

    /// Returns the fetch URL of the given remote, or `None` if it doesn't exist.
    pub fn remote_url(self, name: &str) -> Option<String> {
        self.cmd().args(["remote", "get-url", name]).get_stdout_lossy().ok()
    }

    /// Sets the branch for a submodule.
    pub fn set_submodule_branch(self, rel_path: &Path, branch: &str) -> Result<()> {
        self.cmd()
            .args(["--literal-pathspecs", "submodule", "set-branch", "-b", branch, "--"])
            .arg(rel_path)
            .exec()
            .map(drop)
    }

    /// Returns remote branch names as a newline-separated string.
    pub fn remote_branches(self) -> Result<String> {
        self.cmd().args(["branch", "-r"]).get_stdout_lossy()
    }

    /// Fetches a branch from origin and checks out a local tracking branch at the given path.
    pub fn fetch_and_checkout_branch(self, at: &Path, branch: &str) -> Result<()> {
        self.cmd_at(at).args(["fetch", "origin", branch]).exec().map_err(|e| {
            eyre::eyre!(
                "Could not fetch latest changes for branch {branch} in submodule at {}: {e}",
                at.display()
            )
        })?;
        self.cmd_at(at)
            .args(["checkout", "-B", branch, &format!("origin/{branch}")])
            .exec()
            .map_err(|e| {
                eyre::eyre!(
                    "Could not checkout and track origin/{branch} for submodule at {}: {e}",
                    at.display()
                )
            })?;
        Ok(())
    }

    fn cmd(self) -> Command {
        let mut cmd = Self::cmd_no_root();
        cmd.current_dir(self.root);
        cmd
    }

    fn cmd_at(self, path: &Path) -> Command {
        let mut cmd = Self::cmd_no_root();
        cmd.current_dir(path);
        cmd
    }

    fn cmd_no_root() -> Command {
        let mut cmd = Command::new("git");
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    // don't set this in cmd() because it's not wanted for all commands
    fn stderr(self) -> Stdio {
        if self.quiet { Stdio::piped() } else { Stdio::inherit() }
    }
}

/// A submodule section in `.gitmodules` resolved by its worktree path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleMapping {
    /// Section name used for local config and the module Git directory.
    pub name: String,
    /// Registered submodule URL.
    pub url: Option<String>,
}

/// Deserialized `git submodule status lib/dep` output.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct Submodule {
    /// Current commit hash the submodule is checked out at.
    rev: String,
    /// Relative path to the submodule.
    path: PathBuf,
}

impl Submodule {
    pub const fn new(rev: String, path: PathBuf) -> Self {
        Self { rev, path }
    }

    pub fn rev(&self) -> &str {
        &self.rev
    }

    pub const fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl FromStr for Submodule {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self> {
        let caps = SUBMODULE_STATUS_REGEX
            .captures(s)
            .ok_or_else(|| eyre::eyre!("Invalid submodule status format"))?;

        Ok(Self {
            rev: caps.get(1).unwrap().as_str().to_string(),
            path: PathBuf::from(caps.get(2).unwrap().as_str()),
        })
    }
}

/// Deserialized `git submodule status` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submodules(pub Vec<Submodule>);

impl Submodules {
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromStr for Submodules {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self> {
        let subs = s.lines().map(str::parse).collect::<Result<Vec<Submodule>>>()?;
        Ok(Self(subs))
    }
}

impl<'a> IntoIterator for &'a Submodules {
    type Item = &'a Submodule;
    type IntoIter = std::slice::Iter<'a, Submodule>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use foundry_common::fs;
    use std::{env, fs::File, io::Write};
    use tempfile::tempdir;

    #[test]
    fn parse_submodule_status() {
        let s = "+8829465a08cac423dcf59852f21e448449c1a1a8 lib/openzeppelin-contracts (v4.8.0-791-g8829465a)";
        let sub = Submodule::from_str(s).unwrap();
        assert_eq!(sub.rev(), "8829465a08cac423dcf59852f21e448449c1a1a8");
        assert_eq!(sub.path(), Path::new("lib/openzeppelin-contracts"));

        let s = "-8829465a08cac423dcf59852f21e448449c1a1a8 lib/openzeppelin-contracts";
        let sub = Submodule::from_str(s).unwrap();
        assert_eq!(sub.rev(), "8829465a08cac423dcf59852f21e448449c1a1a8");
        assert_eq!(sub.path(), Path::new("lib/openzeppelin-contracts"));

        let s = "8829465a08cac423dcf59852f21e448449c1a1a8 lib/openzeppelin-contracts";
        let sub = Submodule::from_str(s).unwrap();
        assert_eq!(sub.rev(), "8829465a08cac423dcf59852f21e448449c1a1a8");
        assert_eq!(sub.path(), Path::new("lib/openzeppelin-contracts"));
    }

    #[test]
    fn parse_multiline_submodule_status() {
        let s = r#"+d3db4ef90a72b7d24aa5a2e5c649593eaef7801d lib/forge-std (v1.9.4-6-gd3db4ef)
+8829465a08cac423dcf59852f21e448449c1a1a8 lib/openzeppelin-contracts (v4.8.0-791-g8829465a)
"#;
        let subs = Submodules::from_str(s).unwrap().0;
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].rev(), "d3db4ef90a72b7d24aa5a2e5c649593eaef7801d");
        assert_eq!(subs[0].path(), Path::new("lib/forge-std"));
        assert_eq!(subs[1].rev(), "8829465a08cac423dcf59852f21e448449c1a1a8");
        assert_eq!(subs[1].path(), Path::new("lib/openzeppelin-contracts"));
    }

    #[test]
    fn skips_submodule_status_if_dependencies_are_initialized() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".gitmodules"),
            r#"[submodule "lib/forge-std"]
	path = lib/forge-std
	url = https://github.com/foundry-rs/forge-std
"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("lib/forge-std/.git")).unwrap();

        let git = Git::new(root);
        assert!(git.submodules_initialized(&["lib".into()]).unwrap());
        assert!(git.submodules_initialized(&[root.join("lib").into()]).unwrap());

        let nested = root.join("packages/contracts");
        std::fs::create_dir_all(&nested).unwrap();
        assert!(!Git::new(&nested).submodules_initialized(&["../../lib".into()]).unwrap());

        // The fast path succeeds even though this is not a real Git repository.
        assert!(!git.has_missing_dependencies(["lib"]).unwrap());

        std::fs::remove_dir(root.join("lib/forge-std/.git")).unwrap();
        assert!(!git.submodules_initialized(&["lib".into()]).unwrap());
    }

    #[test]
    fn foundry_path_ext_works() {
        let p = Path::new("contracts/MyTest.t.sol");
        assert!(p.is_sol_test());
        assert!(p.is_sol());
        let p = Path::new("contracts/Greeter.sol");
        assert!(!p.is_sol_test());
    }

    #[test]
    fn parse_ether_value_accepts_hex_prefixed_wei() {
        assert_eq!(parse_ether_value("0x10").unwrap(), U256::from(16));
        assert_eq!(parse_ether_value("0X10").unwrap(), U256::from(16));
        assert_eq!(parse_ether_value("0x12").unwrap(), U256::from(0x12));
        assert_eq!(parse_ether_value("0xff").unwrap(), U256::from(0xff));
        assert_eq!(parse_ether_value("100").unwrap(), U256::from(100));
        assert_eq!(parse_ether_value("1ether").unwrap(), U256::from(1000000000000000000u128));
    }

    // loads .env from cwd and project dir, See [`find_project_root()`]
    #[test]
    fn can_load_dotenv() {
        let temp = tempdir().unwrap();
        Git::new(temp.path()).init().unwrap();
        let cwd_env = temp.path().join(".env");
        fs::create_file(temp.path().join("foundry.toml")).unwrap();
        let nested = temp.path().join("nested");
        fs::create_dir(&nested).unwrap();

        let mut cwd_file = File::create(cwd_env).unwrap();
        let mut prj_file = File::create(nested.join(".env")).unwrap();

        cwd_file.write_all(b"TESTCWDKEY=cwd_val").unwrap();
        cwd_file.sync_all().unwrap();

        prj_file.write_all(b"TESTPRJKEY=prj_val").unwrap();
        prj_file.sync_all().unwrap();

        let cwd = env::current_dir().unwrap();
        env::set_current_dir(nested).unwrap();
        load_dotenv();
        env::set_current_dir(cwd).unwrap();

        assert_eq!(env::var("TESTCWDKEY").unwrap(), "cwd_val");
        assert_eq!(env::var("TESTPRJKEY").unwrap(), "prj_val");
    }

    #[test]
    fn test_read_gitmodules_regex() {
        let gitmodules = r#"
        [submodule "lib/solady"]
        path = lib/solady
        url = ""
        branch = v0.1.0
        [submodule "lib/openzeppelin-contracts"]
        path = lib/openzeppelin-contracts
        url = ""
        branch = v4.8.0-791-g8829465a
        [submodule "lib/forge-std"]
        path = lib/forge-std
        url = ""
"#;

        let paths = SUBMODULE_BRANCH_REGEX
            .captures_iter(gitmodules)
            .map(|cap| {
                (
                    PathBuf::from_str(cap.get(1).unwrap().as_str()).unwrap(),
                    String::from(cap.get(2).unwrap().as_str()),
                )
            })
            .collect::<HashMap<_, _>>();

        assert_eq!(paths.get(Path::new("lib/solady")).unwrap(), "v0.1.0");
        assert_eq!(
            paths.get(Path::new("lib/openzeppelin-contracts")).unwrap(),
            "v4.8.0-791-g8829465a"
        );

        let no_branch_gitmodules = r#"
        [submodule "lib/solady"]
        path = lib/solady
        url = ""
        [submodule "lib/openzeppelin-contracts"]
        path = lib/openzeppelin-contracts
        url = ""
        [submodule "lib/forge-std"]
        path = lib/forge-std
        url = ""
"#;
        let paths = SUBMODULE_BRANCH_REGEX
            .captures_iter(no_branch_gitmodules)
            .map(|cap| {
                (
                    PathBuf::from_str(cap.get(1).unwrap().as_str()).unwrap(),
                    String::from(cap.get(2).unwrap().as_str()),
                )
            })
            .collect::<HashMap<_, _>>();

        assert!(paths.is_empty());

        let branch_in_between = r#"
        [submodule "lib/solady"]
        path = lib/solady
        url = ""
        [submodule "lib/openzeppelin-contracts"]
        path = lib/openzeppelin-contracts
        url = ""
        branch = v4.8.0-791-g8829465a
        [submodule "lib/forge-std"]
        path = lib/forge-std
        url = ""
        "#;

        let paths = SUBMODULE_BRANCH_REGEX
            .captures_iter(branch_in_between)
            .map(|cap| {
                (
                    PathBuf::from_str(cap.get(1).unwrap().as_str()).unwrap(),
                    String::from(cap.get(2).unwrap().as_str()),
                )
            })
            .collect::<HashMap<_, _>>();

        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths.get(Path::new("lib/openzeppelin-contracts")).unwrap(),
            "v4.8.0-791-g8829465a"
        );
    }
}
