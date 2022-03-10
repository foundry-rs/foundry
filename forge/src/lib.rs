pub mod decode;

/// Call trace arena, decoding and formatting
pub mod trace;

/// The Forge test runner
mod runner;
pub use runner::{ContractRunner, TestKind, TestKindGas, TestResult};

/// Forge test runners for multiple contracts
mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

/// Forge test execution backends
pub mod executor;
pub use executor::abi;

pub trait TestFilter {
    fn matches_test(&self, test_name: impl AsRef<str>) -> bool;
    fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool;
    fn matches_path(&self, path: impl AsRef<str>) -> bool;
}

use ethers::types::Address;
use once_cell::sync::Lazy;
static CALLER: Lazy<Address> = Lazy::new(Address::random);

#[cfg(test)]
pub mod test_helpers {
    use crate::executor::fuzz::FuzzedExecutor;

    use super::{
        executor::{
            builder::Backend,
            opts::{Env, EvmOpts},
            Executor, ExecutorBuilder,
        },
        *,
    };
    use ethers::{
        prelude::Lazy,
        solc::{AggregatedCompilerOutput, Project, ProjectPathsConfig},
        types::U256,
    };
    use revm::db::DatabaseRef;

    pub static COMPILED: Lazy<AggregatedCompilerOutput> = Lazy::new(|| {
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        project.compile().unwrap().output()
    });

    pub static EVM_OPTS: Lazy<EvmOpts> = Lazy::new(|| EvmOpts {
        env: Env { gas_limit: 18446744073709551615, chain_id: Some(99), ..Default::default() },
        initial_balance: U256::MAX,
        ..Default::default()
    });

    pub fn test_executor() -> Executor<Backend> {
        ExecutorBuilder::new().with_cheatcodes(false).with_config((*EVM_OPTS).evm_env()).build()
    }

    pub fn fuzz_executor<'a, DB: DatabaseRef>(
        executor: &'a Executor<DB>,
    ) -> FuzzedExecutor<'a, DB> {
        let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };

        FuzzedExecutor::new(executor, proptest::test_runner::TestRunner::new(cfg), *CALLER)
    }

    pub mod filter {
        use super::*;
        use regex::Regex;

        pub struct Filter {
            test_regex: Regex,
            contract_regex: Regex,
            path_regex: Regex,
        }

        impl Filter {
            pub fn new(test_pattern: &str, contract_pattern: &str, path_pattern: &str) -> Self {
                Filter {
                    test_regex: Regex::new(test_pattern).unwrap(),
                    contract_regex: Regex::new(contract_pattern).unwrap(),
                    path_regex: Regex::new(path_pattern).unwrap(),
                }
            }

            pub fn matches_all() -> Self {
                Filter {
                    test_regex: Regex::new(".*").unwrap(),
                    contract_regex: Regex::new(".*").unwrap(),
                    path_regex: Regex::new(".*").unwrap(),
                }
            }
        }

        impl TestFilter for Filter {
            fn matches_test(&self, test_name: impl AsRef<str>) -> bool {
                self.test_regex.is_match(test_name.as_ref())
            }

            fn matches_contract(&self, contract_name: impl AsRef<str>) -> bool {
                self.contract_regex.is_match(contract_name.as_ref())
            }

            fn matches_path(&self, path: impl AsRef<str>) -> bool {
                self.path_regex.is_match(path.as_ref())
            }
        }
    }
}
