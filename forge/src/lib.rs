mod runner;
pub use runner::{ContractRunner, TestResult};

mod multi_runner;
pub use multi_runner::{MultiContractRunner, MultiContractRunnerBuilder};

use ethers::abi;
use eyre::Result;

pub fn decode_revert(error: &[u8]) -> Result<String> {
    Ok(abi::decode(&[abi::ParamType::String], &error[4..])?[0].to_string())
}

#[cfg(test)]
pub mod test_helpers {
    use ethers::{
        prelude::Lazy,
        solc::{CompilerOutput, Project, ProjectPathsConfig},
    };

    pub static COMPILED: Lazy<CompilerOutput> = Lazy::new(|| {
        // NB: should we add a test-helper function that makes creating these
        // ephemeral projects easier?
        let paths =
            ProjectPathsConfig::builder().root("testdata").sources("testdata").build().unwrap();
        let project = Project::builder().paths(paths).ephemeral().no_artifacts().build().unwrap();
        project.compile().unwrap().output()
    });
}
