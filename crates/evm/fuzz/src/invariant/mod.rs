use alloy_json_abi::{Function, JsonAbi};
use alloy_primitives::{Address, Selector, map::HashMap};
use foundry_compilers::artifacts::StorageLayout;
use itertools::Either;
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};

mod call_override;
pub use call_override::RandomCallGenerator;

mod filters;
use crate::BasicTxDetails;
pub use filters::{ArtifactFilters, SenderFilters};
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::utils::{StateChangeset, get_function};

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
