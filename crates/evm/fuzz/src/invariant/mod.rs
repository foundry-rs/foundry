use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Selector, map::HashMap};
use foundry_compilers::artifacts::StorageLayout;
use itertools::Either;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, sync::Arc};

mod call_override;
pub use call_override::RandomCallGenerator;

mod filters;
use crate::BasicTxDetails;
pub use filters::{ArtifactFilters, SenderFilters};
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::utils::{StateChangeset, get_function};

/// Returns true if the function returns `int256`, indicating optimization mode.
/// In optimization mode, the fuzzer maximizes the return value instead of checking invariants.
pub fn is_optimization_invariant(func: &Function) -> bool {
    func.outputs.len() == 1 && func.outputs[0].ty == "int256"
}

/// Contracts identified as targets during a fuzz run.
///
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
    /// Creates a new `FuzzRunIdentifiedContracts` instance.
    pub fn new(targets: TargetedContracts, is_updatable: bool) -> Self {
        Self { targets: Arc::new(Mutex::new(targets)), is_updatable }
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
        if !self.is_updatable {
            return Ok(());
        }

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
            let contract = TargetedContract {
                identifier: artifact.name.clone(),
                abi: contract.abi.clone(),
                targeted_functions: functions,
                excluded_functions: Vec::new(),
                storage_layout: contract.storage_layout.as_ref().map(Arc::clone),
            };
            targets.insert(*address, contract);
        }
        Ok(())
    }

    /// Clears targeted contracts created during an invariant run.
    pub fn clear_created_contracts(&self, created_contracts: Vec<Address>) {
        if !created_contracts.is_empty() {
            let mut targets = self.targets.lock();
            for addr in &created_contracts {
                targets.remove(addr);
            }
        }
    }
}

/// A collection of contracts identified as targets for invariant testing.
#[derive(Clone, Debug, Default)]
pub struct TargetedContracts {
    /// The inner map of targeted contracts.
    pub inner: BTreeMap<Address, TargetedContract>,
}

impl TargetedContracts {
    /// Returns a new `TargetedContracts` instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns fuzzed contract abi and fuzzed function from address and provided calldata.
    ///
    /// Used to decode return values and logs in order to add values into fuzz dictionary.
    pub fn fuzzed_artifacts(&self, tx: &BasicTxDetails) -> (Option<&JsonAbi>, Option<&Function>) {
        match self.inner.get(&tx.call_details.target) {
            Some(c) => (
                Some(&c.abi),
                c.abi.functions().find(|f| f.selector() == tx.call_details.calldata[..4]),
            ),
            None => (None, None),
        }
    }

    /// Returns flatten target contract address and functions to be fuzzed.
    /// Includes contract targeted functions if specified, else all mutable contract functions.
    pub fn fuzzed_functions(&self) -> impl Iterator<Item = (&Address, &Function)> {
        self.inner
            .iter()
            .filter(|(_, c)| !c.abi.functions.is_empty())
            .flat_map(|(contract, c)| c.abi_fuzzed_functions().map(move |f| (contract, f)))
    }

    /// Returns whether the given transaction can be replayed or not with known contracts.
    pub fn can_replay(&self, tx: &BasicTxDetails) -> bool {
        match self.inner.get(&tx.call_details.target) {
            Some(c) => c.abi.functions().any(|f| f.selector() == tx.call_details.calldata[..4]),
            None => false,
        }
    }

    /// Identifies fuzzed contract and function based on given tx details and returns unique metric
    /// key composed from contract identifier and function name.
    pub fn fuzzed_metric_key(&self, tx: &BasicTxDetails) -> Option<String> {
        self.inner.get(&tx.call_details.target).and_then(|contract| {
            contract
                .abi
                .functions()
                .find(|f| f.selector() == tx.call_details.calldata[..4])
                .map(|function| format!("{}.{}", contract.identifier.clone(), function.name))
        })
    }

    /// Returns a map of contract addresses to their storage layouts.
    pub fn get_storage_layouts(&self) -> HashMap<Address, Arc<StorageLayout>> {
        self.inner
            .iter()
            .filter_map(|(addr, c)| {
                c.storage_layout.as_ref().map(|layout| (*addr, Arc::clone(layout)))
            })
            .collect()
    }
}

impl std::ops::Deref for TargetedContracts {
    type Target = BTreeMap<Address, TargetedContract>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for TargetedContracts {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// A contract identified as target for invariant testing.
#[derive(Clone, Debug)]
pub struct TargetedContract {
    /// The contract identifier. This is only used in error messages.
    pub identifier: String,
    /// The contract's ABI.
    pub abi: JsonAbi,
    /// The targeted functions of the contract.
    pub targeted_functions: Vec<Function>,
    /// The excluded functions of the contract.
    pub excluded_functions: Vec<Function>,
    /// The contract's storage layout, if available.
    pub storage_layout: Option<Arc<StorageLayout>>,
}

impl TargetedContract {
    /// Returns a new `TargetedContract` instance.
    pub fn new(identifier: String, abi: JsonAbi) -> Self {
        Self {
            identifier,
            abi,
            targeted_functions: Vec::new(),
            excluded_functions: Vec::new(),
            storage_layout: None,
        }
    }

    /// Determines contract storage layout from project contracts. Needs `storageLayout` to be
    /// enabled as extra output in project configuration.
    pub fn with_project_contracts(mut self, project_contracts: &ContractsByArtifact) -> Self {
        if let Some((src, name)) = self.identifier.split_once(':')
            && let Some((_, contract_data)) = project_contracts.iter().find(|(artifact, _)| {
                artifact.name == name && artifact.source.as_path().ends_with(src)
            })
        {
            self.storage_layout = contract_data.storage_layout.as_ref().map(Arc::clone);
        }
        self
    }

    /// Helper to retrieve functions to fuzz for specified abi.
    /// Returns specified targeted functions if any, else mutable abi functions that are not
    /// marked as excluded.
    pub fn abi_fuzzed_functions(&self) -> impl Iterator<Item = &Function> {
        if !self.targeted_functions.is_empty() {
            Either::Left(self.targeted_functions.iter())
        } else {
            Either::Right(self.abi.functions().filter(|&func| {
                !matches!(
                    func.state_mutability,
                    alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
                ) && !self.excluded_functions.contains(func)
            }))
        }
    }

    /// Returns the function for the given selector.
    pub fn get_function(&self, selector: Selector) -> eyre::Result<&Function> {
        get_function(&self.identifier, selector, &self.abi)
    }

    /// Adds the specified selectors to the targeted functions.
    pub fn add_selectors(
        &mut self,
        selectors: impl IntoIterator<Item = Selector>,
        should_exclude: bool,
    ) -> eyre::Result<()> {
        for selector in selectors {
            if should_exclude {
                self.excluded_functions.push(self.get_function(selector)?.clone());
            } else {
                self.targeted_functions.push(self.get_function(selector)?.clone());
            }
        }
        Ok(())
    }
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

impl<'a> InvariantContract<'a> {
    /// Creates a new invariant contract.
    pub fn new(
        address: Address,
        invariant_function: &'a Function,
        call_after_invariant: bool,
        abi: &'a JsonAbi,
    ) -> Self {
        Self { address, invariant_function, call_after_invariant, abi }
    }

    /// Returns true if this is an optimization mode invariant (returns int256).
    pub fn is_optimization(&self) -> bool {
        is_optimization_invariant(self.invariant_function)
    }
}

/// Settings that determine the validity of a persisted invariant counterexample.
///
/// When a counterexample is replayed, it's only valid if the same contracts, selectors,
/// senders, and fail_on_revert settings are used. Changes to unrelated code (e.g., adding
/// a log statement) should not invalidate the counterexample.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantSettings {
    /// Target contracts with their addresses and identifiers.
    pub target_contracts: BTreeMap<Address, String>,
    /// Target selectors per contract address.
    pub target_selectors: BTreeMap<Address, Vec<Selector>>,
    /// Target senders for the invariant test.
    pub target_senders: Vec<Address>,
    /// Excluded senders for the invariant test.
    pub excluded_senders: Vec<Address>,
    /// Whether the test should fail on any revert.
    pub fail_on_revert: bool,
}

impl InvariantSettings {
    /// Creates new invariant settings from the given components.
    pub fn new(
        targeted_contracts: &TargetedContracts,
        sender_filters: &SenderFilters,
        fail_on_revert: bool,
    ) -> Self {
        let target_contracts = targeted_contracts
            .inner
            .iter()
            .map(|(addr, contract)| (*addr, contract.identifier.clone()))
            .collect();

        let target_selectors = targeted_contracts
            .inner
            .iter()
            .map(|(addr, contract)| {
                let selectors: Vec<Selector> =
                    contract.abi_fuzzed_functions().map(|f| f.selector()).collect();
                (*addr, selectors)
            })
            .collect();

        let mut target_senders = sender_filters.targeted.clone();
        target_senders.sort();

        let mut excluded_senders = sender_filters.excluded.clone();
        excluded_senders.sort();

        Self {
            target_contracts,
            target_selectors,
            target_senders,
            excluded_senders,
            fail_on_revert,
        }
    }

    /// Compares these settings with another and returns a description of what changed.
    /// Returns `None` if the settings are equivalent.
    pub fn diff(&self, other: &Self) -> Option<String> {
        let mut changes = Vec::new();

        if self.target_contracts != other.target_contracts {
            let added: Vec<_> = other
                .target_contracts
                .iter()
                .filter(|(addr, _)| !self.target_contracts.contains_key(*addr))
                .map(|(_, name)| name.as_str())
                .collect();
            let removed: Vec<_> = self
                .target_contracts
                .iter()
                .filter(|(addr, _)| !other.target_contracts.contains_key(*addr))
                .map(|(_, name)| name.as_str())
                .collect();

            if !added.is_empty() {
                changes.push(format!("added target contracts: {}", added.join(", ")));
            }
            if !removed.is_empty() {
                changes.push(format!("removed target contracts: {}", removed.join(", ")));
            }
        }

        if self.target_selectors != other.target_selectors {
            changes.push("target selectors changed".to_string());
        }

        if self.target_senders != other.target_senders {
            changes.push("target senders changed".to_string());
        }

        if self.excluded_senders != other.excluded_senders {
            changes.push("excluded senders changed".to_string());
        }

        if self.fail_on_revert != other.fail_on_revert {
            changes.push(format!(
                "fail_on_revert changed from {} to {}",
                self.fail_on_revert, other.fail_on_revert
            ));
        }

        if changes.is_empty() { None } else { Some(changes.join(", ")) }
    }
}

impl fmt::Display for InvariantSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "targets: {}, selectors: {}, senders: {}, excluded: {}, fail_on_revert: {}",
            self.target_contracts.len(),
            self.target_selectors.values().map(|v| v.len()).sum::<usize>(),
            self.target_senders.len(),
            self.excluded_senders.len(),
            self.fail_on_revert
        )
    }
}
