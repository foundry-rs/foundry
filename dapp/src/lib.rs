mod solc;
use solc::SolcBuilder;

mod executor;
pub use executor::{Executor, MemoryState};

mod artifacts;
pub use artifacts::DapptoolsArtifact;

mod runner;
pub use runner::{ContractRunner, TestResult};

mod multi_runner;
pub use multi_runner::MultiContractRunner;

/// Re-export of the Rust EVM for convenience
pub use evm;

use ethers::{abi, types::U256};
use eyre::Result;

const BASE_TX_COST: u64 = 21000;

fn remove_extra_costs(gas: U256, calldata: &[u8]) -> U256 {
    let mut calldata_cost = 0;
    for i in calldata {
        if *i != 0 {
            // TODO: Check if EVM pre-eip2028 and charge 64
            calldata_cost += 16
        } else {
            calldata_cost += 8;
        }
    }
    gas - calldata_cost - BASE_TX_COST
}

pub fn decode_revert(error: &[u8]) -> Result<String> {
    Ok(abi::decode(&[abi::ParamType::String], &error[4..])?[0].to_string())
}

#[cfg(test)]
use ethers::prelude::Lazy;
#[cfg(test)]
use ethers::utils::CompiledContract;
#[cfg(test)]
use std::collections::HashMap;

#[cfg(test)]
static COMPILED: Lazy<HashMap<String, CompiledContract>> = Lazy::new(|| {
    SolcBuilder::new("./*.sol", &[], &[])
        .unwrap()
        .build_all()
        .unwrap()
});
