mod artifacts;
pub use artifacts::DapptoolsArtifact;

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

    use ethers::{prelude::Lazy, utils::CompiledContract};
    use std::collections::HashMap;

    use dapp_solc::SolcBuilder;

    pub static COMPILED: Lazy<HashMap<String, CompiledContract>> =
        Lazy::new(|| SolcBuilder::new("./*.sol", &[], &[]).unwrap().build_all().unwrap());
}
