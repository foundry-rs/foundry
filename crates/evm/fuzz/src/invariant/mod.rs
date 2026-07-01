use alloy_json_abi::{Event, Function, JsonAbi};
use alloy_primitives::{Address, B256, Selector, map::HashMap};
use foundry_compilers::artifacts::StorageLayout;
use itertools::Either;
use serde::{Deserialize, Serialize};
use std::{
    cell::{Cell, Ref, RefCell},
    collections::BTreeMap,
    fmt,
    rc::Rc,
    sync::Arc,
};

mod call_override;
pub use call_override::RandomCallGenerator;

mod filters;
use crate::BasicTxDetails;
pub use filters::{ArtifactFilters, SenderFilters};
use foundry_common::{ContractsByAddress, ContractsByArtifact};
use foundry_evm_core::utils::StateChangeset;

type DynamicTargetCacheKey = (Address, B256);
type DynamicTargetArtifactMatchCache =
    Rc<RefCell<HashMap<DynamicTargetCacheKey, Option<CachedTargetContract>>>>;
type FuzzedFunction = (Address, Function);
type FunctionLookup = HashMap<Selector, Function>;

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
    targets: Rc<RefCell<TargetedContracts>>,
    /// Flat cache of all currently fuzzable target functions.
    fuzzed_functions: Rc<RefCell<Vec<FuzzedFunction>>>,
    /// Generation counter for cached fuzzed functions.
    fuzzed_functions_generation: Rc<Cell<u64>>,
    /// Whether target contracts are updatable or not.
    pub is_updatable: bool,
    artifact_matches: DynamicTargetArtifactMatchCache,
}

impl FuzzRunIdentifiedContracts {
    /// Creates a new `FuzzRunIdentifiedContracts` instance.
    pub fn new(targets: TargetedContracts, is_updatable: bool) -> Self {
        let fuzzed_functions = Self::flatten_fuzzed_functions(&targets);
        Self {
            targets: Rc::new(RefCell::new(targets)),
            fuzzed_functions: Rc::new(RefCell::new(fuzzed_functions)),
            fuzzed_functions_generation: Rc::new(Cell::new(0)),
            is_updatable,
            artifact_matches: Rc::new(RefCell::new(HashMap::default())),
        }
    }

    /// Borrows the current targeted contracts.
    pub fn targets(&self) -> Ref<'_, TargetedContracts> {
        self.targets.borrow()
    }

    /// Borrows the current flat list of fuzzed target functions.
    pub fn fuzzed_functions(&self) -> Ref<'_, [FuzzedFunction]> {
        Ref::map(self.fuzzed_functions.borrow(), Vec::as_slice)
    }

    /// Returns the current fuzzed-functions generation.
    pub fn fuzzed_functions_generation(&self) -> u64 {
        self.fuzzed_functions_generation.get()
    }

    fn refresh_fuzzed_functions(&self) {
        let fuzzed_functions = {
            let targets = self.targets.borrow();
            Self::flatten_fuzzed_functions(&targets)
        };
        *self.fuzzed_functions.borrow_mut() = fuzzed_functions;
        self.fuzzed_functions_generation.set(self.fuzzed_functions_generation.get() + 1);
    }

    fn flatten_fuzzed_functions(targets: &TargetedContracts) -> Vec<FuzzedFunction> {
        targets.fuzzed_functions().map(|(address, function)| (*address, function.clone())).collect()
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

        let mut targets_changed = false;
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
            let code_hash = code.hash_slow();
            let code = code.original_byte_slice();
            let Some(contract) = self.target_contract_for_code(
                *address,
                code_hash,
                code,
                project_contracts,
                artifact_filters,
            )?
            else {
                continue;
            };
            created_contracts.push(*address);
            self.targets.borrow_mut().insert(*address, contract.into_targeted_contract());
            targets_changed = true;
        }
        if targets_changed {
            self.refresh_fuzzed_functions();
        }
        Ok(())
    }

    fn target_contract_for_code(
        &self,
        address: Address,
        code_hash: B256,
        code: &[u8],
        project_contracts: &ContractsByArtifact,
        artifact_filters: &ArtifactFilters,
    ) -> eyre::Result<Option<CachedTargetContract>> {
        let cache_key = (address, code_hash);
        if let Some(cached_match) = self.artifact_matches.borrow().get(&cache_key) {
            return Ok(cached_match.clone());
        }

        let cached_match = if let Some((artifact, contract_data)) =
            project_contracts.find_by_deployed_code(code)
        {
            artifact_filters.get_targeted_functions(artifact, &contract_data.abi)?.map(
                |targeted_functions| CachedTargetContract {
                    identifier: artifact.name.clone(),
                    abi: contract_data.abi.clone(),
                    targeted_functions,
                    storage_layout: contract_data.storage_layout.as_ref().map(Arc::clone),
                    event_lookup: Arc::new(TargetedContractEvents::new(&contract_data.abi)),
                },
            )
        } else {
            None
        };
        self.artifact_matches.borrow_mut().insert(cache_key, cached_match.clone());
        Ok(cached_match)
    }

    /// Clears targeted contracts created during an invariant run.
    pub fn clear_created_contracts(&self, created_contracts: Vec<Address>) {
        let mut targets_changed = false;
        if !created_contracts.is_empty() {
            let mut targets = self.targets.borrow_mut();
            for addr in &created_contracts {
                targets_changed |= targets.remove(addr).is_some();
            }
        }
        if targets_changed {
            self.refresh_fuzzed_functions();
        }
    }
}

#[derive(Clone, Debug)]
struct CachedTargetContract {
    identifier: String,
    abi: JsonAbi,
    targeted_functions: Vec<Function>,
    storage_layout: Option<Arc<StorageLayout>>,
    event_lookup: Arc<TargetedContractEvents>,
}

impl CachedTargetContract {
    fn into_targeted_contract(self) -> TargetedContract {
        TargetedContract::from_parts(
            self.identifier,
            self.abi,
            self.targeted_functions,
            Vec::new(),
            self.storage_layout,
            self.event_lookup,
        )
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

    /// Returns fuzzed contract and fuzzed function from address and provided calldata.
    ///
    /// Used to decode return values and logs in order to add values into fuzz dictionary.
    pub fn fuzzed_artifacts(
        &self,
        tx: &BasicTxDetails,
    ) -> (Option<&TargetedContract>, Option<&Function>) {
        match self.inner.get(&tx.call_details.target) {
            Some(c) => {
                let function = tx
                    .call_details
                    .calldata
                    .get(..4)
                    .and_then(|selector| <[u8; 4]>::try_from(selector).ok())
                    .map(Selector::from)
                    .and_then(|selector| c.function_by_selector(selector));
                (Some(c), function)
            }
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
            Some(c) => tx
                .call_details
                .calldata
                .get(..4)
                .and_then(|selector| <[u8; 4]>::try_from(selector).ok())
                .map(Selector::from)
                .is_some_and(|selector| c.fuzzed_function_by_selector(selector).is_some()),
            None => false,
        }
    }

    /// Identifies fuzzed contract and function based on given tx details and returns unique metric
    /// key composed from contract identifier and function name.
    pub fn fuzzed_metric_key(&self, tx: &BasicTxDetails) -> Option<String> {
        tx.call_details
            .calldata
            .get(..4)
            .and_then(|selector| <[u8; 4]>::try_from(selector).ok())
            .map(Selector::from)
            .and_then(|selector| {
                self.fuzzed_metric_key_for_selector(tx.call_details.target, selector)
            })
    }

    /// Identifies fuzzed contract and function from target and selector and returns unique metric
    /// key composed from contract identifier and function name.
    pub fn fuzzed_metric_key_for_selector(
        &self,
        target: Address,
        selector: Selector,
    ) -> Option<String> {
        self.inner.get(&target).and_then(|contract| {
            contract
                .function_by_selector(selector)
                .map(|function| format!("{}.{}", contract.identifier.as_str(), function.name))
        })
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
    /// Contract events indexed by topic0 and indexed-topic count for log dictionary decoding.
    pub event_lookup: Arc<TargetedContractEvents>,
    functions_by_selector: FunctionLookup,
    fuzzed_functions_by_selector: FunctionLookup,
}

impl TargetedContract {
    /// Returns a new `TargetedContract` instance.
    pub fn new(identifier: String, abi: JsonAbi) -> Self {
        let event_lookup = Arc::new(TargetedContractEvents::new(&abi));
        Self::from_parts(identifier, abi, Vec::new(), Vec::new(), None, event_lookup)
    }

    fn from_parts(
        identifier: String,
        abi: JsonAbi,
        targeted_functions: Vec<Function>,
        excluded_functions: Vec<Function>,
        storage_layout: Option<Arc<StorageLayout>>,
        event_lookup: Arc<TargetedContractEvents>,
    ) -> Self {
        let mut contract = Self {
            identifier,
            abi,
            targeted_functions,
            excluded_functions,
            storage_layout,
            event_lookup,
            functions_by_selector: FunctionLookup::default(),
            fuzzed_functions_by_selector: FunctionLookup::default(),
        };
        contract.rebuild_function_lookups();
        contract
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
    /// Returns specified targeted functions if any, else mutable abi functions, always skipping
    /// functions marked as excluded.
    pub fn abi_fuzzed_functions(&self) -> impl Iterator<Item = &Function> {
        if self.targeted_functions.is_empty() {
            Either::Right(self.abi.functions().filter(|&func| {
                !matches!(
                    func.state_mutability,
                    alloy_json_abi::StateMutability::Pure | alloy_json_abi::StateMutability::View
                ) && !self.excluded_functions.contains(func)
            }))
        } else {
            Either::Left(
                self.targeted_functions
                    .iter()
                    .filter(|func| !self.excluded_functions.contains(func)),
            )
        }
    }

    pub fn rebuild_function_lookups(&mut self) {
        let functions_by_selector =
            self.abi.functions().fold(FunctionLookup::default(), |mut functions, function| {
                functions.entry(function.selector()).or_insert_with(|| function.clone());
                functions
            });
        let fuzzed_functions_by_selector = self.abi_fuzzed_functions().fold(
            FunctionLookup::default(),
            |mut functions, function| {
                functions.entry(function.selector()).or_insert_with(|| function.clone());
                functions
            },
        );
        self.functions_by_selector = functions_by_selector;
        self.fuzzed_functions_by_selector = fuzzed_functions_by_selector;
    }

    /// Returns any ABI function for the given selector.
    pub fn function_by_selector(&self, selector: Selector) -> Option<&Function> {
        self.functions_by_selector.get(&selector)
    }

    /// Returns a fuzzable function for the given selector.
    pub fn fuzzed_function_by_selector(&self, selector: Selector) -> Option<&Function> {
        self.fuzzed_functions_by_selector.get(&selector)
    }

    /// Returns the function for the given selector.
    pub fn get_function(&self, selector: Selector) -> eyre::Result<&Function> {
        self.function_by_selector(selector)
            .ok_or_else(|| eyre::eyre!("{} does not have the selector {selector}", self.identifier))
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
        self.rebuild_function_lookups();
        Ok(())
    }
}

/// Events for a targeted contract, pre-indexed for log dictionary decoding.
#[derive(Clone, Debug, Default)]
pub struct TargetedContractEvents {
    by_topic: HashMap<(B256, usize), Vec<TargetedContractEvent>>,
    anonymous: Vec<TargetedContractEvent>,
}

impl TargetedContractEvents {
    fn new(abi: &JsonAbi) -> Self {
        let mut events = Self::default();
        for (order, event) in abi.events().enumerate() {
            let event = TargetedContractEvent { order, event: event.clone() };
            if event.event.anonymous {
                events.anonymous.push(event);
            } else {
                let indexed_count = event.event.inputs.iter().filter(|input| input.indexed).count();
                events
                    .by_topic
                    .entry((event.event.selector(), indexed_count))
                    .or_default()
                    .push(event);
            }
        }
        events
    }

    pub fn by_topic(
        &self,
        selector: &B256,
        indexed_count: usize,
    ) -> Option<&[TargetedContractEvent]> {
        self.by_topic.get(&(*selector, indexed_count)).map(Vec::as_slice)
    }

    pub fn anonymous(&self) -> &[TargetedContractEvent] {
        &self.anonymous
    }
}

/// Event with its flattened ABI order for preserving log decode priority.
#[derive(Clone, Debug)]
pub struct TargetedContractEvent {
    event: Event,
    order: usize,
}

impl TargetedContractEvent {
    pub const fn order(&self) -> usize {
        self.order
    }

    pub const fn event(&self) -> &Event {
        &self.event
    }
}

/// Test contract which is testing its invariants.
#[derive(Clone, Debug)]
pub struct InvariantContract<'a> {
    /// Address of the test contract.
    pub address: Address,
    /// Name of the test contract.
    pub name: &'a str,
    /// Invariant functions to assert against, paired with their `fail_on_revert` config.
    /// Stored in **source declaration order** so failure-event attribution and report
    /// rendering match user expectations.
    pub invariant_fns: Vec<(&'a Function, bool)>,
    /// Index into [`Self::invariant_fns`] of the stable campaign anchor. Boolean invariant
    /// suites use a deterministic contract-local anchor so test filters do not affect
    /// corpus/failure namespaces.
    pub anchor_idx: usize,
    /// If true, `afterInvariant` function is called after each invariant run.
    pub call_after_invariant: bool,
    /// ABI of the test contract.
    pub abi: &'a JsonAbi,
}

impl<'a> InvariantContract<'a> {
    /// Creates a new invariant contract.
    ///
    /// Caller must ensure `invariant_fns` is non-empty and `anchor_idx < invariant_fns.len()`.
    pub const fn new(
        address: Address,
        name: &'a str,
        invariant_fns: Vec<(&'a Function, bool)>,
        anchor_idx: usize,
        call_after_invariant: bool,
        abi: &'a JsonAbi,
    ) -> Self {
        Self { address, name, invariant_fns, anchor_idx, call_after_invariant, abi }
    }

    /// Returns the stable campaign anchor.
    pub fn anchor(&self) -> &'a Function {
        self.invariant_fns[self.anchor_idx].0
    }

    /// Returns true if this is an optimization mode invariant (returns int256).
    pub fn is_optimization(&self) -> bool {
        is_optimization_invariant(self.anchor())
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
            self.fail_on_revert,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CallDetails;
    use alloy_primitives::{Bytes, U256};
    use foundry_compilers::{
        ArtifactId,
        artifacts::{
            BytecodeObject, CompactBytecode, CompactContractBytecode, CompactDeployedBytecode,
        },
    };
    use revm::{bytecode::Bytecode, state::Account};

    fn abi_with_functions(functions: &[&str]) -> JsonAbi {
        let mut abi = JsonAbi::new();
        for function in functions {
            let function = Function::parse(function).unwrap();
            abi.functions.entry(function.name.clone()).or_default().push(function);
        }
        abi
    }

    fn targeted_contracts_with_functions(target: Address, functions: &[&str]) -> TargetedContracts {
        let mut targets = TargetedContracts::new();
        targets.inner.insert(
            target,
            TargetedContract::new("Target".to_string(), abi_with_functions(functions)),
        );
        targets
    }

    fn targeted_contracts_with_function(target: Address, function: Function) -> TargetedContracts {
        let mut abi = JsonAbi::new();
        abi.functions.entry(function.name.clone()).or_default().push(function);
        let mut targets = TargetedContracts::new();
        targets.inner.insert(target, TargetedContract::new("Target".to_string(), abi));
        targets
    }

    fn tx(target: Address, calldata: impl Into<Bytes>) -> BasicTxDetails {
        BasicTxDetails {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            call_details: CallDetails { target, calldata: calldata.into(), value: None },
        }
    }

    fn artifact_id(name: &str) -> ArtifactId {
        ArtifactId {
            path: format!("{name}.json").into(),
            name: name.to_string(),
            source: format!("{name}.sol").into(),
            version: "0.8.30".parse().unwrap(),
            build_id: "test".to_string(),
            profile: "test".to_string(),
        }
    }

    fn project_contracts_with_runtime_code(name: &str, code: Bytes) -> ContractsByArtifact {
        project_contracts_with_runtime_code_and_abi(name, code, JsonAbi::new())
    }

    fn project_contracts_with_runtime_code_and_abi(
        name: &str,
        code: Bytes,
        abi: JsonAbi,
    ) -> ContractsByArtifact {
        let deployed_bytecode = CompactDeployedBytecode {
            bytecode: Some(CompactBytecode {
                object: BytecodeObject::Bytecode(code),
                source_map: None,
                link_references: Default::default(),
            }),
            immutable_references: Default::default(),
        };
        let artifact = CompactContractBytecode {
            abi: Some(abi),
            bytecode: None,
            deployed_bytecode: Some(deployed_bytecode),
        };
        ContractsByArtifact::new([(artifact_id(name), artifact)])
    }

    fn touched_account_with_code(code: Bytes) -> Account {
        let mut account = Account::default();
        account.info.balance = U256::ZERO;
        account.info.code = Some(Bytecode::new_raw(code));
        account.mark_touch();
        account
    }

    #[test]
    fn targeted_contracts_short_calldata_is_not_replayable_or_decodable() {
        let target = Address::from([0x42; 20]);
        let targets = targeted_contracts_with_function(target, Function::parse("foo()").unwrap());
        let tx = tx(target, vec![0xde, 0xad, 0xbe]);

        assert!(!targets.can_replay(&tx));
        assert!(targets.fuzzed_artifacts(&tx).1.is_none());
        assert!(targets.fuzzed_metric_key(&tx).is_none());
    }

    #[test]
    fn abi_fuzzed_functions_filters_excluded_targeted_functions() {
        let allowed = Function::parse("allowed()").unwrap();
        let excluded = Function::parse("excluded()").unwrap();
        let mut contract = TargetedContract::new("Target".to_string(), JsonAbi::new());
        contract.targeted_functions = vec![allowed.clone(), excluded.clone()];
        contract.excluded_functions = vec![excluded];

        let selectors = contract.abi_fuzzed_functions().map(Function::selector).collect::<Vec<_>>();

        assert_eq!(selectors, vec![allowed.selector()]);
    }

    #[test]
    fn targeted_contracts_refresh_selector_lookup_after_filters() {
        let target = Address::from([0x42; 20]);
        let foo = Function::parse("foo()").unwrap();
        let bar = Function::parse("bar()").unwrap();

        let mut excluded = targeted_contracts_with_functions(target, &["foo()", "bar()"]);
        excluded.inner.get_mut(&target).unwrap().add_selectors([foo.selector()], true).unwrap();
        assert!(!excluded.can_replay(&tx(target, foo.selector().to_vec())));
        assert!(excluded.can_replay(&tx(target, bar.selector().to_vec())));
        assert_eq!(
            excluded.fuzzed_artifacts(&tx(target, foo.selector().to_vec())).1.unwrap().name,
            "foo"
        );

        let mut targeted = targeted_contracts_with_functions(target, &["foo()", "bar()"]);
        targeted.inner.get_mut(&target).unwrap().add_selectors([foo.selector()], false).unwrap();
        assert!(targeted.can_replay(&tx(target, foo.selector().to_vec())));
        assert!(!targeted.can_replay(&tx(target, bar.selector().to_vec())));
        assert_eq!(
            targeted.fuzzed_metric_key_for_selector(target, bar.selector()).unwrap(),
            "Target.bar"
        );
    }

    #[test]
    fn fuzz_run_identified_contracts_cache_fuzzed_functions_in_target_order() {
        let first = Address::from([0x01; 20]);
        let second = Address::from([0x02; 20]);
        let mut targets = targeted_contracts_with_functions(second, &["bar()", "baz(uint256)"]);
        targets.inner.insert(
            first,
            TargetedContract::new(
                "First".to_string(),
                abi_with_functions(&["foo()", "qux(address)"]),
            ),
        );
        let expected = targets
            .fuzzed_functions()
            .map(|(address, function)| (*address, function.selector()))
            .collect::<Vec<_>>();

        let identified = FuzzRunIdentifiedContracts::new(targets, true);
        let actual = identified
            .fuzzed_functions()
            .iter()
            .map(|(address, function)| (*address, function.selector()))
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[test]
    fn collect_created_contracts_caches_deployed_code_matches() {
        let existing = Address::from([0x42; 20]);
        let created = Address::from([0x43; 20]);
        let setup = Address::from([0x44; 20]);
        let untouched = Address::from([0x45; 20]);
        let runtime_code = Bytes::from_static(&[0x60, 0x00, 0x56]);
        let project_contracts =
            project_contracts_with_runtime_code("DynamicTarget", runtime_code.clone());
        let mut targets = TargetedContracts::new();
        for address in [existing, setup, untouched] {
            targets.inner.insert(
                address,
                TargetedContract::new("AlreadyTargeted".to_string(), JsonAbi::new()),
            );
        }
        let identified = FuzzRunIdentifiedContracts::new(targets, true);

        let mut state_changeset = StateChangeset::default();
        state_changeset.insert(existing, touched_account_with_code(runtime_code.clone()));
        state_changeset.insert(setup, touched_account_with_code(runtime_code.clone()));
        state_changeset.insert(untouched, Account::default());
        state_changeset.insert(created, touched_account_with_code(runtime_code));
        let mut created_contracts = Vec::new();
        let setup_contracts =
            ContractsByAddress::from([(setup, ("Setup".to_string(), JsonAbi::new()))]);

        identified
            .collect_created_contracts(
                &state_changeset,
                &project_contracts,
                &setup_contracts,
                &ArtifactFilters::default(),
                &mut created_contracts,
            )
            .unwrap();

        created_contracts.sort_unstable();
        assert_eq!(created_contracts, vec![existing, created]);
        let targets = identified.targets();
        assert_eq!(targets[&existing].identifier, "DynamicTarget");
        assert_eq!(targets[&created].identifier, "DynamicTarget");
        assert_eq!(targets[&setup].identifier, "AlreadyTargeted");
        assert_eq!(targets[&untouched].identifier, "AlreadyTargeted");
        drop(targets);

        identified
            .collect_created_contracts(
                &state_changeset,
                &Default::default(),
                &setup_contracts,
                &ArtifactFilters::default(),
                &mut created_contracts,
            )
            .unwrap();

        created_contracts.sort_unstable();
        assert_eq!(created_contracts, vec![existing, existing, created, created]);
    }

    #[test]
    fn collect_and_clear_created_contracts_refresh_fuzzed_function_cache() {
        let existing = Address::from([0x42; 20]);
        let created = Address::from([0x43; 20]);
        let runtime_code = Bytes::from_static(&[0x60, 0x00, 0x56]);
        let project_contracts = project_contracts_with_runtime_code_and_abi(
            "DynamicTarget",
            runtime_code.clone(),
            abi_with_functions(&["dynamic(uint256)"]),
        );
        let identified = FuzzRunIdentifiedContracts::new(
            targeted_contracts_with_functions(existing, &["existing()"]),
            true,
        );

        let initial = identified
            .fuzzed_functions()
            .iter()
            .map(|(address, function)| (*address, function.selector()))
            .collect::<Vec<_>>();
        assert_eq!(initial, vec![(existing, Function::parse("existing()").unwrap().selector())]);
        assert_eq!(identified.fuzzed_functions_generation(), 0);

        let mut state_changeset = StateChangeset::default();
        state_changeset.insert(created, touched_account_with_code(runtime_code));
        let mut created_contracts = Vec::new();

        identified
            .collect_created_contracts(
                &state_changeset,
                &project_contracts,
                &ContractsByAddress::default(),
                &ArtifactFilters::default(),
                &mut created_contracts,
            )
            .unwrap();

        let with_created = identified
            .fuzzed_functions()
            .iter()
            .map(|(address, function)| (*address, function.selector()))
            .collect::<Vec<_>>();
        assert_eq!(identified.fuzzed_functions_generation(), 1);
        assert_eq!(
            with_created,
            vec![
                (existing, Function::parse("existing()").unwrap().selector()),
                (created, Function::parse("dynamic(uint256)").unwrap().selector()),
            ]
        );

        identified.clear_created_contracts(created_contracts);
        let cleared = identified
            .fuzzed_functions()
            .iter()
            .map(|(address, function)| (*address, function.selector()))
            .collect::<Vec<_>>();
        assert_eq!(cleared, initial);
        assert_eq!(identified.fuzzed_functions_generation(), 2);
    }
}
