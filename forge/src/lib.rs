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
    };
    use regex::Regex;

    pub static COMPILED: Lazy<CompilerOutput> = Lazy::new(|| {
        // NB: should we add a test-helper function that makes creating these
        // ephemeral projects easier?
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        project.compile().unwrap().output()
    });

    pub struct Filter {
        test_regex: Regex,
        contract_regex: Regex,
    }

    impl Filter {
        pub fn new(test_pattern: &str, contract_pattern: &str) -> Self {
            return Filter {
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
