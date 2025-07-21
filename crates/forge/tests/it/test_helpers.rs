//! Test helpers for Forge integration tests.

use alloy_chains::NamedChain;
use alloy_primitives::U256;
use forge::{MultiContractRunner, MultiContractRunnerBuilder};
use foundry_cli::utils::install_crypto_provider;
use foundry_compilers::{
    Project, ProjectCompileOutput, SolcConfig, Vyper,
    artifacts::{EvmVersion, Libraries, Settings},
    compilers::multi::MultiCompiler,
    utils::RuntimeOrHandle,
};
use foundry_config::{
    Config, FsPermissions, FuzzConfig, FuzzDictionaryConfig, InvariantConfig, RpcEndpointUrl,
    RpcEndpoints, fs_permissions::PathPermission,
};
use foundry_evm::{constants::CALLER, opts::EvmOpts};
use foundry_test_utils::{
    fd_lock, init_tracing,
    rpc::{next_http_archive_rpc_url, next_rpc_endpoint},
};
use revm::primitives::hardfork::SpecId;
use std::{
    env, fmt,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

pub const RE_PATH_SEPARATOR: &str = "/";
const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata");
static VYPER: LazyLock<PathBuf> = LazyLock::new(|| std::env::temp_dir().join("vyper"));

/// Profile for the tests group. Used to configure separate configurations for test runs.
pub enum ForgeTestProfile {
    Default,
    Paris,
    MultiVersion,
}

impl fmt::Display for ForgeTestProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Paris => write!(f, "paris"),
            Self::MultiVersion => write!(f, "multi-version"),
        }
    }
}

impl ForgeTestProfile {
    /// Returns true if the profile is Paris.
    pub fn is_paris(&self) -> bool {
        matches!(self, Self::Paris)
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

        if matches!(self, Self::Paris) {
            settings.evm_version = Some(EvmVersion::Paris);
        }

        let settings = SolcConfig::builder().settings(settings).build();
        SolcConfig { settings }
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

        config.prompt_timeout = 0;

        config.optimizer = Some(true);
        config.optimizer_runs = Some(200);

        config.gas_limit = u64::MAX.into();
        config.chain = None;
        config.tx_origin = CALLER;
        config.block_number = U256::from(1);
        config.block_timestamp = U256::from(1);

        config.sender = CALLER;
        config.initial_balance = U256::MAX;
        config.ffi = true;
        config.verbosity = 3;
        config.memory_limit = 1 << 26;

        if self.is_paris() {
            config.evm_version = EvmVersion::Paris;
        }

        config.fuzz = FuzzConfig {
            runs: 256,
            fail_on_revert: true,
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
            failure_persist_dir: Some(tempfile::tempdir().unwrap().keep()),
            failure_persist_file: Some("testfailure".to_string()),
            show_logs: false,
            timeout: None,
        };
        config.invariant = InvariantConfig {
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
            corpus_dir: None,
            corpus_gzip: true,
            corpus_min_mutations: 5,
            corpus_min_size: 0,
            failure_persist_dir: Some(
                tempfile::Builder::new()
                    .prefix(&format!("foundry-{self}"))
                    .tempdir()
                    .unwrap()
                    .keep(),
            ),
            show_metrics: true,
            timeout: None,
            show_solidity: false,
            show_edge_coverage: false,
        };

        config.sanitized()
    }
}

/// Container for test data for a specific test profile.
pub struct ForgeTestData {
    pub project: Project,
    pub output: ProjectCompileOutput,
    pub config: Arc<Config>,
    pub profile: ForgeTestProfile,
}

impl ForgeTestData {
    /// Builds [ForgeTestData] for the given [ForgeTestProfile].
    ///
    /// Uses [get_compiled] to lazily compile the project.
    pub fn new(profile: ForgeTestProfile) -> Self {
        install_crypto_provider();
        init_tracing();
        let config = Arc::new(profile.config());
        let mut project = config.project().unwrap();
        let output = get_compiled(&mut project);
        Self { project, output, config, profile }
    }

    /// Builds a base runner
    pub fn base_runner(&self) -> MultiContractRunnerBuilder {
        init_tracing();
        let config = self.config.clone();
        let mut runner = MultiContractRunnerBuilder::new(config).sender(self.config.sender);
        if self.profile.is_paris() {
            runner = runner.evm_spec(SpecId::MERGE);
        }
        runner
    }

    /// Builds a non-tracing runner
    pub fn runner(&self) -> MultiContractRunner {
        self.runner_with(|_| {})
    }

    /// Builds a non-tracing runner
    pub fn runner_with(&self, modify: impl FnOnce(&mut Config)) -> MultiContractRunner {
        let mut config = (*self.config).clone();
        modify(&mut config);
        self.runner_with_config(config)
    }

    fn runner_with_config(&self, mut config: Config) -> MultiContractRunner {
        config.rpc_endpoints = rpc_endpoints();
        config.allow_paths.push(manifest_root().to_path_buf());

        if config.fs_permissions.is_empty() {
            config.fs_permissions =
                FsPermissions::new(vec![PathPermission::read_write(manifest_root())]);
        }

        let opts = config_evm_opts(&config);

        let mut builder = self.base_runner();
        let config = Arc::new(config);
        let root = self.project.root();
        builder.config = config.clone();
        builder
            .enable_isolation(opts.isolate)
            .sender(config.sender)
            .build::<MultiCompiler>(root, &self.output, opts.local_evm_env(), opts)
            .unwrap()
    }

    /// Builds a tracing runner
    pub fn tracing_runner(&self) -> MultiContractRunner {
        let mut opts = config_evm_opts(&self.config);
        opts.verbosity = 5;
        self.base_runner()
            .build::<MultiCompiler>(self.project.root(), &self.output, opts.local_evm_env(), opts)
            .unwrap()
    }

    /// Builds a runner that runs against forked state
    pub async fn forked_runner(&self, rpc: &str) -> MultiContractRunner {
        let mut opts = config_evm_opts(&self.config);

        opts.env.chain_id = None; // clear chain id so the correct one gets fetched from the RPC
        opts.fork_url = Some(rpc.to_string());

        let env = opts.evm_env().await.expect("Could not instantiate fork environment");
        let fork = opts.get_fork(&Default::default(), env.clone());

        self.base_runner()
            .with_fork(fork)
            .build::<MultiCompiler>(self.project.root(), &self.output, env, opts)
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
        let url = format!("https://github.com/vyperlang/vyper/releases/download/v0.4.3/vyper.0.4.3+commit.bff19ea2.{suffix}");

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
pub static TEST_DATA_DEFAULT: LazyLock<ForgeTestData> =
    LazyLock::new(|| ForgeTestData::new(ForgeTestProfile::Default));

/// Data for tests requiring Paris support on Solc and EVM level.
pub static TEST_DATA_PARIS: LazyLock<ForgeTestData> =
    LazyLock::new(|| ForgeTestData::new(ForgeTestProfile::Paris));

/// Data for tests requiring Prague support on Solc and EVM level.
pub static TEST_DATA_MULTI_VERSION: LazyLock<ForgeTestData> =
    LazyLock::new(|| ForgeTestData::new(ForgeTestProfile::MultiVersion));

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
        ("mainnet", RpcEndpointUrl::Url(next_http_archive_rpc_url())),
        ("mainnet2", RpcEndpointUrl::Url(next_http_archive_rpc_url())),
        ("sepolia", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Sepolia))),
        ("optimism", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Optimism))),
        ("arbitrum", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Arbitrum))),
        ("polygon", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::Polygon))),
        ("bsc", RpcEndpointUrl::Url(next_rpc_endpoint(NamedChain::BinanceSmartChain))),
        ("avaxTestnet", RpcEndpointUrl::Url("https://api.avax-test.network/ext/bc/C/rpc".into())),
        ("moonbeam", RpcEndpointUrl::Url("https://moonbeam-rpc.publicnode.com".into())),
        ("rpcEnvAlias", RpcEndpointUrl::Env("${RPC_ENV_ALIAS}".into())),
    ])
}

fn config_evm_opts(config: &Config) -> EvmOpts {
    config.to_figment(foundry_config::FigmentProviders::None).extract().unwrap()
}
