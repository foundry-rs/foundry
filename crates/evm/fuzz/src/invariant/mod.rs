use alloy_json_abi::{Function, JsonAbi as Abi};
use alloy_primitives::{Address, Bytes};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};

mod call_override;
pub use call_override::RandomCallGenerator;

mod filters;
pub use filters::{ArtifactFilters, SenderFilters};

pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;
pub type FuzzRunIdentifiedContracts = Arc<Mutex<TargetedContracts>>;

/// (Sender, (TargetContract, Calldata))
pub type BasicTxDetails = (Address, (Address, Bytes));

/// Test contract which is testing its invariants.
#[derive(Debug, Clone)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Invariant function present in the test contract.
    pub invariant_function: &'a Function,
    /// Abi of the test contract.
    pub abi: &'a Abi,
}
