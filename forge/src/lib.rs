mod runner;
pub use runner::{ContractRunner, TestKind, TestKindGas, TestResult};

mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

pub trait TestFilter {
    fn matches_test(&self, test_name: &str) -> bool;
    fn matches_contract(&self, contract_name: &str) -> bool;
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;
    use ethers::{
        prelude::Lazy,
        solc::{CompilerOutput, Project, ProjectPathsConfig},
        types::U256,
    };
    use evm_adapters::{
        evm_opts::{Env, EvmOpts, EvmType},
        sputnik::helpers::VICINITY,
        FAUCET_ACCOUNT,
    };
    use regex::Regex;
    use sputnik::backend::MemoryBackend;

    pub static COMPILED: Lazy<CompilerOutput> = Lazy::new(|| {
        // NB: should we add a test-helper function that makes creating these
        // ephemeral projects easier?
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        project.compile().unwrap().output()
    });

    pub static EVM_OPTS: Lazy<EvmOpts> = Lazy::new(|| EvmOpts {
        env: Env { gas_limit: 18446744073709551615, chain_id: Some(1), ..Default::default() },
        initial_balance: U256::MAX,
        evm_type: EvmType::Sputnik,
        ..Default::default()
    });

    pub static BACKEND: Lazy<MemoryBackend<'static>> = Lazy::new(|| {
        let mut backend = MemoryBackend::new(&*VICINITY, Default::default());
        // max out the balance of the faucet
        let faucet = backend.state_mut().entry(*FAUCET_ACCOUNT).or_insert_with(Default::default);
        faucet.balance = U256::MAX;
        backend
    });

    pub struct Filter {
        test_regex: Regex,
        contract_regex: Regex,
    }

    impl Filter {
        pub fn new(test_pattern: &str, contract_pattern: &str) -> Self {
            Filter {
                test_regex: Regex::new(test_pattern).unwrap(),
                contract_regex: Regex::new(contract_pattern).unwrap(),
            }
        }
    }

    impl TestFilter for Filter {
        fn matches_test(&self, test_name: &str) -> bool {
            self.test_regex.is_match(test_name)
        }

        fn matches_contract(&self, contract_name: &str) -> bool {
            self.contract_regex.is_match(contract_name)
        }
    }
}
