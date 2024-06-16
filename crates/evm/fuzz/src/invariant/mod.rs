use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Bytes};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};

mod call_override;
pub use call_override::RandomCallGenerator;

mod filters;
pub use filters::{ArtifactFilters, SenderFilters};
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::utils::StateChangeset;

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

    /// Returns fuzzed contract abi and fuzzed function from address and provided calldata.
    ///
    /// Used to decode return values and logs in order to add values into fuzz dictionary.
    pub fn with_fuzzed_artifacts(
        &self,
        tx: &BasicTxDetails,
        f: impl FnOnce(Option<&JsonAbi>, Option<&Function>),
    ) {
        let targets = self.targets.lock();
        let (abi, abi_f) = match targets.get(&tx.call_details.target) {
            Some((_, abi, _)) => {
                (Some(abi), abi.functions().find(|f| f.selector() == tx.call_details.calldata[..4]))
            }
            None => (None, None),
        };
        f(abi, abi_f);
    }

    /// Returns flatten target contract address and functions to be fuzzed.
    /// Includes contract targeted functions if specified, else all mutable contract functions.
    pub fn fuzzed_functions(&self) -> Vec<(Address, Function)> {
        let mut fuzzed_functions = vec![];
        for (contract, (_, abi, functions)) in self.targets.lock().iter() {
            if !abi.functions.is_empty() {
                for function in abi_fuzzed_functions(abi, functions) {
                    fuzzed_functions.push((*contract, function.clone()));
                }
            }
        }
        fuzzed_functions
    }

    /// If targets are updatable, collect all contracts created during an invariant run (which
    /// haven't been discovered yet).
    pub fn collect_created_contracts(
        &self,
        state_changeset: &StateChangeset,
        project_contracts: &ContractsByArtifact,
        setup_contracts: &ContractsByAddress,
        artifact_filters: &ArtifactFilters,
        created_contracts: &mut Vec<Address>,
    ) -> eyre::Result<()> {
        if self.is_updatable {
            let mut targets = self.targets.lock();
            for (address, account) in state_changeset {
                if setup_contracts.contains_key(address) {
                    continue;
                }
                if !account.is_touched() {
                    continue;
                }
                let Some(code) = &account.info.code else {
                    continue;
                };
                if code.is_empty() {
                    continue;
                }
                let Some((artifact, contract)) =
                    project_contracts.find_by_deployed_code(code.original_byte_slice())
                else {
                    continue;
                };
                let Some(functions) =
                    artifact_filters.get_targeted_functions(artifact, &contract.abi)?
                else {
                    continue;
                };
                created_contracts.push(*address);
                targets.insert(*address, (artifact.name.clone(), contract.abi.clone(), functions));
            }
        }
        Ok(())
    }

    /// Clears targeted contracts created during an invariant run.
    pub fn clear_created_contracts(&self, created_contracts: Vec<Address>) {
        if !created_contracts.is_empty() {
            let mut targets = self.targets.lock();
            for addr in created_contracts.iter() {
                targets.remove(addr);
            }
        }
    }
}

/// Helper to retrieve functions to fuzz for specified abi.
/// Returns specified targeted functions if any, else mutable abi functions.
pub(crate) fn abi_fuzzed_functions(
    abi: &JsonAbi,
    targeted_functions: &[Function],
) -> Vec<Function> {
    if !targeted_functions.is_empty() {
        targeted_functions.to_vec()
    } else {
        abi.functions()
            .filter(|&func| {
                !matches!(
                    func.state_mutability,
                    alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
                )
            })
            .cloned()
            .collect()
    }
}

/// Details of a transaction generated by invariant strategy for fuzzing a target.
#[derive(Clone, Debug)]
pub struct BasicTxDetails {
    // Transaction sender address.
    pub sender: Address,
    // Transaction call details.
    pub call_details: CallDetails,
}

/// Call details of a transaction generated to fuzz invariant target.
#[derive(Clone, Debug)]
pub struct CallDetails {
    // Address of target contract.
    pub target: Address,
    // The data of the transaction.
    pub calldata: Bytes,
}

/// Test contract which is testing its invariants.
#[derive(Clone, Debug)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Invariant function present in the test contract.
    pub invariant_function: &'a Function,
    /// If true, `afterInvariant` function is called after each invariant run.
    pub call_after_invariant: bool,
    /// ABI of the test contract.
    pub abi: &'a JsonAbi,
}
