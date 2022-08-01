#![allow(unused)]

use super::*;
use ethers::{
    prelude::{artifacts::Settings, Lazy, ProjectCompileOutput, SolcConfig},
    solc::{artifacts::Libraries, utils::RuntimeOrHandle, Project, ProjectPathsConfig},
    types::{Address, U256},
};
use foundry_evm::{
    executor::{
        backend::Backend,
        opts::{Env, EvmOpts},
        DatabaseRef, Executor, ExecutorBuilder,
    },
    fuzz::FuzzedExecutor,
    CALLER,
};
use std::{path::PathBuf, str::FromStr};

pub static PROJECT: Lazy<Project> = Lazy::new(|| {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata");
    let paths = ProjectPathsConfig::builder().root(root.clone()).sources(root).build().unwrap();
    Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap()
});

pub static LIBS_PROJECT: Lazy<Project> = Lazy::new(|| {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testdata");
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
        eprintln!("{}", out);
        panic!("Compiled with errors");
    }
    out
});

pub static COMPILED_WITH_LIBS: Lazy<ProjectCompileOutput> = Lazy::new(|| {
    let out = (*LIBS_PROJECT).compile().unwrap();
    if out.has_compiler_errors() {
        eprintln!("{}", out);
        panic!("Compiled with errors");
    }
    out
});

pub static EVM_OPTS: Lazy<EvmOpts> = Lazy::new(|| EvmOpts {
    env: Env {
        gas_limit: 18446744073709551615,
        chain_id: Some(foundry_common::DEV_CHAIN_ID),
        tx_origin: Address::from_str("00a329c0648769a73afac7f9381e08fb43dbea72").unwrap(),
        block_number: 1,
        block_timestamp: 1,
        ..Default::default()
    },
    sender: Address::from_str("00a329c0648769a73afac7f9381e08fb43dbea72").unwrap(),
    initial_balance: U256::MAX,
    ffi: true,
    memory_limit: 2u64.pow(24),
    ..Default::default()
});

pub fn fuzz_executor<DB: DatabaseRef>(executor: &Executor) -> FuzzedExecutor {
    let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };

    FuzzedExecutor::new(executor, proptest::test_runner::TestRunner::new(cfg), CALLER)
}

#[cfg(not(windows))]
pub const RE_PATH_SEPARATOR: &str = "/";

#[cfg(windows)]
pub const RE_PATH_SEPARATOR: &str = "\\\\";

pub mod filter {
    use super::*;
    use regex::Regex;

    pub struct Filter {
        test_regex: Regex,
        contract_regex: Regex,
        path_regex: Regex,
        exclude_tests: Option<Regex>,
    }

    impl Filter {
        pub fn new(test_pattern: &str, contract_pattern: &str, path_pattern: &str) -> Self {
            Filter {
                test_regex: Regex::new(test_pattern).unwrap(),
                contract_regex: Regex::new(contract_pattern).unwrap(),
                path_regex: Regex::new(path_pattern).unwrap(),
                exclude_tests: None,
            }
        }

        /// All tests to also exclude
        ///
        /// This is a workaround since regex does not support negative look aheads
        pub fn exclude_tests(mut self, pattern: &str) -> Self {
            self.exclude_tests = Some(Regex::new(pattern).unwrap());
            self
        }

        pub fn matches_all() -> Self {
            Filter {
                test_regex: Regex::new(".*").unwrap(),
                contract_regex: Regex::new(".*").unwrap(),
                path_regex: Regex::new(".*").unwrap(),
                exclude_tests: None,
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
            self.path_regex.is_match(path.as_ref())
        }
    }
}
