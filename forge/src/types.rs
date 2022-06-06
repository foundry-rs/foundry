use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use ethers::abi::{Event, Function, Abi, AbiError};
use ethers::solc::artifacts::CompactContractBytecode;
use ethers::types::H256;

/// Represents a solidity Contract that's a test target
#[derive(Debug, Clone)]
pub struct TestContract {
    /// All functions keyed by their short signature
    pub functions: BTreeMap<[u8; 4], TestFunction>,

    /// contract's bytecode objects
    pub bytecode: CompactContractBytecode,

    /// location of the contract
    pub source: PathBuf,

    /// all events of the contract
    pub events: BTreeMap<H256, Event>,

    /// all errors of the contract
    pub errors: BTreeMap<String, Vec<AbiError>>,
}

/// A solidity function that can be tested
#[derive(Debug, Clone)]
pub struct TestFunction {
    pub function: Function,
    /// the function's signature
    pub signature: String,
}

// === impl TestFunction ===

impl TestFunction {

}