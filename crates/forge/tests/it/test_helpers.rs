#![allow(unused)]

use super::*;
use ethers::{
    prelude::{artifacts::Settings, Lazy, ProjectCompileOutput, SolcConfig},
    solc::{artifacts::Libraries, Project, ProjectPathsConfig},
    types::{Address, U256},
};
use foundry_config::Config;
use foundry_evm::{
    executor::{
        backend::Backend,
        opts::{Env, EvmOpts},
        DatabaseRef, Executor, ExecutorBuilder,
    },
    fuzz::FuzzedExecutor,
    utils::{b160_to_h160, h160_to_b160, u256_to_ru256},
    CALLER,
};
use std::{path::PathBuf, str::FromStr};

pub static PROJECT: Lazy<Project> = Lazy::new(|| {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata");
    let paths = ProjectPathsConfig::builder().root(root.clone()).sources(root).build().unwrap();
    Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap()
});

pub static LIBS_PROJECT: Lazy<Project> = Lazy::new(|| {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata");
    let paths = ProjectPathsConfig::builder().root(root.clone()).sources(root).build().unwrap();
    let libs =
        ["fork/Fork.t.sol:DssExecLib:0xfD88CeE74f7D78697775aBDAE53f9Da1559728E4".to_string()];

    let settings = Settings { libraries: Libraries::parse(&libs).unwrap(), ..Default::default() };

    let solc_config = SolcConfig::builder().settings(settings).build();
    Project::builder()
        .paths(paths)
        .ephemeral()
        .no_artifacts()
        .solc_config(solc_config)
        .build()
        .unwrap()
});

pub static COMPILED: Lazy<ProjectCompileOutput> = Lazy::new(|| {
    let out = (*PROJECT).compile().unwrap();
    if out.has_compiler_errors() {
        eprintln!("{out}");
        panic!("Compiled with errors");
    }
    out
});

pub static COMPILED_WITH_LIBS: Lazy<ProjectCompileOutput> = Lazy::new(|| {
    let out = (*LIBS_PROJECT).compile().unwrap();
    if out.has_compiler_errors() {
        eprintln!("{out}");
        panic!("Compiled with errors");
    }
    out
});

pub static EVM_OPTS: Lazy<EvmOpts> = Lazy::new(|| EvmOpts {
    env: Env {
        gas_limit: 18446744073709551615,
        chain_id: None,
        tx_origin: h160_to_b160(Config::DEFAULT_SENDER),
        block_number: 1,
        block_timestamp: 1,
        ..Default::default()
    },
    sender: h160_to_b160(Config::DEFAULT_SENDER),
    initial_balance: u256_to_ru256(U256::MAX),
    ffi: true,
    memory_limit: 2u64.pow(24),
    ..Default::default()
});

pub fn fuzz_executor<DB: DatabaseRef>(executor: &Executor) -> FuzzedExecutor {
    let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };

    FuzzedExecutor::new(
        executor,
        proptest::test_runner::TestRunner::new(cfg),
        b160_to_h160(CALLER),
        config::test_opts().fuzz,
    )
}

pub const RE_PATH_SEPARATOR: &str = "/";

pub mod filter {
    use super::*;
    use foundry_common::TestFilter;
    use regex::Regex;

    pub struct Filter {
        test_regex: Regex,
        contract_regex: Regex,
        path_regex: Regex,
        exclude_tests: Option<Regex>,
        exclude_paths: Option<Regex>,
    }

    impl Filter {
        pub fn new(test_pattern: &str, contract_pattern: &str, path_pattern: &str) -> Self {
            Filter {
                test_regex: Regex::new(test_pattern)
                    .unwrap_or_else(|_| panic!("Failed to parse test pattern: `{test_pattern}`")),
                contract_regex: Regex::new(contract_pattern).unwrap_or_else(|_| {
                    panic!("Failed to parse contract pattern: `{contract_pattern}`")
                }),
                path_regex: Regex::new(path_pattern)
                    .unwrap_or_else(|_| panic!("Failed to parse path pattern: `{path_pattern}`")),
                exclude_tests: None,
                exclude_paths: None,
            }
        }

        pub fn contract(contract_pattern: &str) -> Self {
            Self::new(".*", contract_pattern, ".*")
        }

        pub fn path(path_pattern: &str) -> Self {
            Self::new(".*", ".*", path_pattern)
        }

        /// All tests to also exclude
        ///
        /// This is a workaround since regex does not support negative look aheads
        pub fn exclude_tests(mut self, pattern: &str) -> Self {
            self.exclude_tests = Some(Regex::new(pattern).unwrap());
            self
        }

        /// All paths to also exclude
        ///
        /// This is a workaround since regex does not support negative look aheads
        pub fn exclude_paths(mut self, pattern: &str) -> Self {
            self.exclude_paths = Some(Regex::new(pattern).unwrap());
            self
        }

        pub fn matches_all() -> Self {
            Filter {
                test_regex: Regex::new(".*").unwrap(),
                contract_regex: Regex::new(".*").unwrap(),
                path_regex: Regex::new(".*").unwrap(),
                exclude_tests: None,
                exclude_paths: None,
            }
        }
    }

    impl TestFilter for Filter {
        fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
            let test_name = test_name.as_ref();
            if let Some(ref exclude) = self.exclude_tests {
                if exclude.is_match(test_name) {
                    return false
                }
            }
            self.test_regex.is_match(test_name)
        }

        fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool {
            self.contract_regex.is_match(contract_name.as_ref())
        }

        fn matches_path(&self, path: impl AsRef<str>) -> bool {
            let path = path.as_ref();
            if let Some(ref exclude) = self.exclude_paths {
                if exclude.is_match(path) {
                    return false
                }
            }
            self.path_regex.is_match(path)
        }
    }
}
