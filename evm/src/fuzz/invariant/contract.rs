use crate::Address;
use ethers::abi::{Contract as Abi, Function};

/// Test contract which is testing its invariants.
#[derive(Debug, Clone)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Invariant functions present in the test contract.
    pub invariant_functions: Vec<&'a Function>,
    /// Abi of the test contract.
    pub abi: &'a Abi,
}
