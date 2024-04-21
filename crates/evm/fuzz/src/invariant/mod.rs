use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};

mod call_override;
pub use call_override::RandomCallGenerator;

mod filters;
pub use filters::{ArtifactFilters, SenderFilters};

pub type TargetedContracts = BTreeMap<Address, (String, JsonAbi, Vec<Function>)>;

/// Contracts identified as targets during a fuzz run.
/// During execution, any newly created contract is added as target and used through the rest of
/// the fuzz run if the collection is updatable (no `targetContract` specified in `setUp`).
#[derive(Clone, Debug)]
pub struct FuzzRunIdentifiedContracts {
    /// Contracts identified as targets during a fuzz run.
    pub targets: Arc<Mutex<TargetedContracts>>,
    /// Whether target contracts are updatable or not.
    pub is_updatable: bool,
}

impl FuzzRunIdentifiedContracts {
    pub fn new(targets: TargetedContracts, is_updatable: bool) -> Self {
        Self { targets: Arc::new(Mutex::new(targets)), is_updatable }
    }
}

/// (Sender, (TargetContract, Calldata, FuzzedFunction, TargetContractAbi))
pub type BasicTxDetails = (Address, (Address, Bytes, Option<Function>, JsonAbi));

/// Test contract which is testing its invariants.
#[derive(Clone, Debug)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Invariant function present in the test contract.
    pub invariant_function: &'a Function,
    /// ABI of the test contract.
    pub abi: &'a JsonAbi,
}
