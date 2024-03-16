//! Test helpers for Forge integration tests.

use alloy_primitives::U256;
use forge::{TestOptions, TestOptionsBuilder};
use foundry_compilers::{
    artifacts::{Libraries, Settings},
    EvmVersion, Project, ProjectCompileOutput, SolcConfig,
};
use foundry_config::{Config, FuzzConfig, FuzzDictionaryConfig, InvariantConfig};
use foundry_evm::{
    constants::CALLER,
    opts::{Env, EvmOpts},
};
use foundry_test_utils::fd_lock;
use once_cell::sync::Lazy;
use std::{
    env, fmt,
    io::Write,
    path::{Path, PathBuf},
};

pub const RE_PATH_SEPARATOR: &str = "/";
const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata");

pub enum ForgeTestProfile {
    Default,
    Cancun,
}

impl fmt::Display for ForgeTestProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeTestProfile::Default => write!(f, "default"),
            ForgeTestProfile::Cancun => write!(f, "cancun"),
        }
    }
}

impl ForgeTestProfile {
    pub fn root(&self) -> PathBuf {
        PathBuf::from(TESTDATA)
    }

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
                    max_calldata_fuzz_dictionary_addresses: 0,
                },
                gas_report_samples: 256,
                failure_persist_dir: Some(tempfile::tempdir().unwrap().into_path()),
                failure_persist_file: Some("testfailure".to_string()),
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
                    max_calldata_fuzz_dictionary_addresses: 0,
                },
                shrink_sequence: true,
                shrink_run_limit: 2usize.pow(18u32),
                preserve_state: false,
                max_assume_rejects: 65536,
                gas_report_samples: 256,
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

    pub fn config(&self) -> Config {
        let mut config = Config::with_root(self.root());

        config.ast = true;
        config.src = self.root().join(self.to_string());
        config.out = self.root().join("out").join(self.to_string());
        config.cache_path = self.root().join("cache").join(self.to_string());
        config.libraries = vec![
            "fork/Fork.t.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string(),
        ];

        if matches!(self, Self::Cancun) {
            config.evm_version = EvmVersion::Cancun;
        }

        config
    }
}

pub struct ForgeTestData {
    pub project: Project,
    pub output: ProjectCompileOutput,
    pub test_opts: TestOptions,
    pub evm_opts: EvmOpts,
    pub config: Config,
}

impl ForgeTestData {
    pub fn new(profile: ForgeTestProfile) -> Self {
        let project = profile.project();
        let output = get_compiled(&project);
        let test_opts = profile.test_opts(&output);
        let config = profile.config();
        let evm_opts = profile.evm_opts();

        Self { project, output, test_opts, evm_opts, config }
    }
}

pub fn get_compiled(project: &Project) -> ProjectCompileOutput {
    let lock_file_path = project.sources_path().join(".lock");
    // Compile only once per test run.
    // We need to use a file lock because `cargo-nextest` runs tests in different processes.
    // This is similar to [`foundry_test_utils::util::initialize`], see its comments for more
    // details.
    let mut lock = fd_lock::new_lock(&lock_file_path);
    let read = lock.read().unwrap();
    let out;
    if project.cache_path().exists() && std::fs::read(&lock_file_path).unwrap() == b"1" {
        out = project.compile();
        drop(read);
    } else {
        drop(read);
        let mut write = lock.write().unwrap();
        write.write_all(b"1").unwrap();
        out = project.compile();
        drop(write);
    }

    let out = out.unwrap();
    if out.has_compiler_errors() {
        panic!("Compiled with errors:\n{out}");
    }
    out
}

pub static TEST_DATA_DEFAULT: Lazy<ForgeTestData> =
    Lazy::new(|| ForgeTestData::new(ForgeTestProfile::Default));
pub static TEST_DATA_CANCUN: Lazy<ForgeTestData> =
    Lazy::new(|| ForgeTestData::new(ForgeTestProfile::Cancun));
