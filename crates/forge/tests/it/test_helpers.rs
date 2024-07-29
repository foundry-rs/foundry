//! Test helpers for Forge integration tests.

use alloy_chains::NamedChain;
use alloy_primitives::U256;
use forge::{
    revm::primitives::SpecId, MultiContractRunner, MultiContractRunnerBuilder, TestOptions,
    TestOptionsBuilder,
};
use foundry_compilers::{
    artifacts::{EvmVersion, Libraries, Settings},
    utils::RuntimeOrHandle,
    Project, ProjectCompileOutput, SolcConfig, Vyper,
};
use foundry_config::{
    fs_permissions::PathPermission, Config, FsPermissions, FuzzConfig, FuzzDictionaryConfig,
    InvariantConfig, RpcEndpoint, RpcEndpoints,
};
use foundry_evm::{
    constants::CALLER,
    opts::{Env, EvmOpts},
};
use foundry_test_utils::{fd_lock, init_tracing, rpc::next_rpc_endpoint};
use once_cell::sync::Lazy;
use std::{
    env, fmt,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const RE_PATH_SEPARATOR: &str = "/";
const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata");
static VYPER: Lazy<PathBuf> = Lazy::new(|| std::env::temp_dir().join("vyper"));

/// Profile for the tests group. Used to configure separate configurations for test runs.
pub enum ForgeTestProfile {
    Default,
    Cancun,
    MultiVersion,
}

impl fmt::Display for ForgeTestProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Cancun => write!(f, "cancun"),
            Self::MultiVersion => write!(f, "multi-version"),
        }
    }
}

impl ForgeTestProfile {
    /// Returns true if the profile is Cancun.
    pub fn is_cancun(&self) -> bool {
        matches!(self, Self::Cancun)
    }

    pub fn root(&self) -> PathBuf {
        PathBuf::from(TESTDATA)
    }

    /// Configures the solc settings for the test profile.
    pub fn solc_config(&self) -> SolcConfig {
        let libs =
            ["fork/Fork.t.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string()];

        let mut settings =
            Settings { libraries: Libraries::parse(&libs).unwrap(), ..Default::default() };

        if matches!(self, Self::Cancun) {
            settings.evm_version = Some(EvmVersion::Cancun);
        }

        SolcConfig::builder().settings(settings).build()
    }

    pub fn project(&self) -> Project {
        self.config().project().expect("Failed to build project")
    }

    pub fn test_opts(&self, output: &ProjectCompileOutput) -> TestOptions {
        TestOptionsBuilder::default()
            .fuzz(FuzzConfig {
                runs: 256,
                max_test_rejects: 65536,
                seed: None,
                dictionary: FuzzDictionaryConfig {
                    include_storage: true,
                    include_push_bytes: true,
                    dictionary_weight: 40,
                    max_fuzz_dictionary_addresses: 10_000,
                    max_fuzz_dictionary_values: 10_000,
                },
                gas_report_samples: 256,
                failure_persist_dir: Some(tempfile::tempdir().unwrap().into_path()),
                failure_persist_file: Some("testfailure".to_string()),
                show_logs: false,
            })
            .invariant(InvariantConfig {
                runs: 256,
                depth: 15,
                fail_on_revert: false,
                call_override: false,
                dictionary: FuzzDictionaryConfig {
                    dictionary_weight: 80,
                    include_storage: true,
                    include_push_bytes: true,
                    max_fuzz_dictionary_addresses: 10_000,
                    max_fuzz_dictionary_values: 10_000,
                },
                shrink_run_limit: 5000,
                max_assume_rejects: 65536,
                gas_report_samples: 256,
                failure_persist_dir: Some(tempfile::tempdir().unwrap().into_path()),
            })
            .build(output, Path::new(self.project().root()))
            .expect("Config loaded")
    }

    pub fn evm_opts(&self) -> EvmOpts {
        EvmOpts {
            env: Env {
                gas_limit: u64::MAX,
                chain_id: None,
                tx_origin: CALLER,
                block_number: 1,
                block_timestamp: 1,
                ..Default::default()
            },
            sender: CALLER,
            initial_balance: U256::MAX,
            ffi: true,
            verbosity: 3,
            memory_limit: 1 << 26,
            ..Default::default()
        }
    }

    /// Build [Config] for test profile.
    ///
    /// Project source files are read from testdata/{profile_name}
    /// Project output files are written to testdata/out/{profile_name}
    /// Cache is written to testdata/cache/{profile_name}
    ///
    /// AST output is enabled by default to support inline configs.
    pub fn config(&self) -> Config {
        let mut config = Config::with_root(self.root());

        config.ast = true;
        config.src = self.root().join(self.to_string());
        config.out = self.root().join("out").join(self.to_string());
        config.cache_path = self.root().join("cache").join(self.to_string());
        config.libraries = vec![
            "fork/Fork.t.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string(),
        ];

        if self.is_cancun() {
            config.evm_version = EvmVersion::Cancun;
        }

        config
    }
}

/// Container for test data for a specific test profile.
pub struct ForgeTestData {
    pub project: Project,
    pub output: ProjectCompileOutput,
    pub test_opts: TestOptions,
    pub evm_opts: EvmOpts,
    pub config: Config,
    pub profile: ForgeTestProfile,
}

impl ForgeTestData {
    /// Builds [ForgeTestData] for the given [ForgeTestProfile].
    ///
    /// Uses [get_compiled] to lazily compile the project.
    pub fn new(profile: ForgeTestProfile) -> Self {
        init_tracing();

        let mut project = profile.project();
        let output = get_compiled(&mut project);
        let test_opts = profile.test_opts(&output);
        let config = profile.config();
        let evm_opts = profile.evm_opts();

        Self { project, output, test_opts, evm_opts, config, profile }
    }

    /// Builds a base runner
    pub fn base_runner(&self) -> MultiContractRunnerBuilder {
        init_tracing();
        let mut runner = MultiContractRunnerBuilder::new(Arc::new(self.config.clone()))
            .sender(self.evm_opts.sender)
            .with_test_options(self.test_opts.clone());
        if self.profile.is_cancun() {
            runner = runner.evm_spec(SpecId::CANCUN);
        }

        runner
    }

    /// Builds a non-tracing runner
    pub fn runner(&self) -> MultiContractRunner {
        let mut config = self.config.clone();
        config.fs_permissions =
            FsPermissions::new(vec![PathPermission::read_write(manifest_root())]);
        self.runner_with_config(config)
    }

    /// Builds a non-tracing runner
    pub fn runner_with_config(&self, mut config: Config) -> MultiContractRunner {
        config.rpc_endpoints = rpc_endpoints();
        config.allow_paths.push(manifest_root().to_path_buf());

        // no prompt testing
        config.prompt_timeout = 0;

        let root = self.project.root();
        let mut opts = self.evm_opts.clone();

        if config.isolate {
            opts.isolate = true;
        }

        let sender = config.sender;

        let mut builder = self.base_runner();
        builder.config = Arc::new(config);
        builder
            .enable_isolation(opts.isolate)
            .sender(sender)
            .with_test_options(self.test_opts.clone())
            .build(root, &self.output, opts.local_evm_env(), opts)
            .unwrap()
    }

    /// Builds a tracing runner
    pub fn tracing_runner(&self) -> MultiContractRunner {
        let mut opts = self.evm_opts.clone();
        opts.verbosity = 5;
        self.base_runner()
            .build(self.project.root(), &self.output, opts.local_evm_env(), opts)
            .unwrap()
    }

    /// Builds a runner that runs against forked state
    pub async fn forked_runner(&self, rpc: &str) -> MultiContractRunner {
        let mut opts = self.evm_opts.clone();

        opts.env.chain_id = None; // clear chain id so the correct one gets fetched from the RPC
        opts.fork_url = Some(rpc.to_string());

        let env = opts.evm_env().await.expect("Could not instantiate fork environment");
        let fork = opts.get_fork(&Default::default(), env.clone());

        self.base_runner()
            .with_fork(fork)
            .build(self.project.root(), &self.output, env, opts)
            .unwrap()
    }
}

/// Installs Vyper if it's not already present.
pub fn get_vyper() -> Vyper {
    if let Ok(vyper) = Vyper::new("vyper") {
        return vyper;
    }
    if let Ok(vyper) = Vyper::new(&*VYPER) {
        return vyper;
    }
    RuntimeOrHandle::new().block_on(async {
        #[cfg(target_family = "unix")]
        use std::{fs::Permissions, os::unix::fs::PermissionsExt};

        let suffix = match svm::platform() {
            svm::Platform::MacOsAarch64 => "darwin",
            svm::Platform::LinuxAmd64 => "linux",
            svm::Platform::WindowsAmd64 => "windows.exe",
            platform => panic!(
                "unsupported platform {platform:?} for installing vyper, \
                 install it manually and add it to $PATH"
            ),
        };
        let url = format!("https://github.com/vyperlang/vyper/releases/download/v0.4.0/vyper.0.4.0+commit.e9db8d9f.{suffix}");

        let res = reqwest::Client::builder().build().unwrap().get(url).send().await.unwrap();

        assert!(res.status().is_success());

        let bytes = res.bytes().await.unwrap();

        std::fs::write(&*VYPER, bytes).unwrap();

        #[cfg(target_family = "unix")]
        std::fs::set_permissions(&*VYPER, Permissions::from_mode(0o755)).unwrap();

        Vyper::new(&*VYPER).unwrap()
    })
}

pub fn get_compiled(project: &mut Project) -> ProjectCompileOutput {
    let lock_file_path = project.sources_path().join(".lock");
    // Compile only once per test run.
    // We need to use a file lock because `cargo-nextest` runs tests in different processes.
    // This is similar to [`foundry_test_utils::util::initialize`], see its comments for more
    // details.
    let mut lock = fd_lock::new_lock(&lock_file_path);
    let read = lock.read().unwrap();
    let out;

    let mut write = None;
    if !project.cache_path().exists() || std::fs::read(&lock_file_path).unwrap() != b"1" {
        drop(read);
        write = Some(lock.write().unwrap());
    }

    if project.compiler.vyper.is_none() {
        project.compiler.vyper = Some(get_vyper());
    }

    out = project.compile().unwrap();

    if out.has_compiler_errors() {
        panic!("Compiled with errors:\n{out}");
    }

    if let Some(ref mut write) = write {
        write.write_all(b"1").unwrap();
    }

    out
}

/// Default data for the tests group.
pub static TEST_DATA_DEFAULT: Lazy<ForgeTestData> =
    Lazy::new(|| ForgeTestData::new(ForgeTestProfile::Default));

/// Data for tests requiring Cancun support on Solc and EVM level.
pub static TEST_DATA_CANCUN: Lazy<ForgeTestData> =
    Lazy::new(|| ForgeTestData::new(ForgeTestProfile::Cancun));

/// Data for tests requiring Cancun support on Solc and EVM level.
pub static TEST_DATA_MULTI_VERSION: Lazy<ForgeTestData> =
    Lazy::new(|| ForgeTestData::new(ForgeTestProfile::MultiVersion));

pub fn manifest_root() -> &'static Path {
    let mut root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // need to check here where we're executing the test from, if in `forge` we need to also allow
    // `testdata`
    if root.ends_with("forge") {
        root = root.parent().unwrap();
    }
    root
}

/// the RPC endpoints used during tests
pub fn rpc_endpoints() -> RpcEndpoints {
    RpcEndpoints::new([
        ("mainnet", RpcEndpoint::Url(next_rpc_endpoint(NamedChain::Mainnet))),
        ("mainnet2", RpcEndpoint::Url(next_rpc_endpoint(NamedChain::Mainnet))),
        ("sepolia", RpcEndpoint::Url(next_rpc_endpoint(NamedChain::Sepolia))),
        ("optimism", RpcEndpoint::Url(next_rpc_endpoint(NamedChain::Optimism))),
        ("arbitrum", RpcEndpoint::Url(next_rpc_endpoint(NamedChain::Arbitrum))),
        ("polygon", RpcEndpoint::Url(next_rpc_endpoint(NamedChain::Polygon))),
        ("avaxTestnet", RpcEndpoint::Url("https://api.avax-test.network/ext/bc/C/rpc".into())),
        ("rpcEnvAlias", RpcEndpoint::Env("${RPC_ENV_ALIAS}".into())),
    ])
}
