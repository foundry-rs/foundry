use crate::{
    executors::{Executor, RawCallResult},
    inspectors::Fuzzer,
};
use alloy_primitives::{Address, FixedBytes, U256};
use alloy_sol_types::{sol, SolCall};
use eyre::{eyre, ContextCompat, Result};
use foundry_common::contracts::{ContractsByAddress, ContractsByArtifact};
use foundry_config::InvariantConfig;
use foundry_evm_core::{
    constants::{CALLER, CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS, MAGIC_ASSUME},
    utils::{get_function, StateChangeset},
};
use foundry_evm_fuzz::{
    invariant::{
        ArtifactFilters, BasicTxDetails, FuzzRunIdentifiedContracts, InvariantContract,
        RandomCallGenerator, SenderFilters, TargetedContracts,
    },
    strategies::{
        build_initial_state, collect_created_contracts, invariant_strat, override_call_strat,
        CalldataFuzzDictionary, EvmFuzzState,
    },
    FuzzCase, FuzzedCases,
};
use foundry_evm_traces::CallTraceArena;
use parking_lot::RwLock;
use proptest::{
    strategy::{BoxedStrategy, Strategy, ValueTree},
    test_runner::{TestCaseError, TestRunner},
};
use revm::{primitives::HashMap, DatabaseCommit};
use std::{borrow::Cow, cell::RefCell, collections::BTreeMap, sync::Arc};

mod error;
use self::error::FailedInvariantCaseData;
pub use error::{InvariantFailures, InvariantFuzzError, InvariantFuzzTestResult};

mod funcs;
pub use funcs::{assert_invariants, replay_run};

sol! {
    interface IInvariantTest {
        #[derive(Default)]
        struct FuzzSelector {
            address addr;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzAbiSelector {
            string contract_abi;
            bytes4[] selectors;
        }

        #[derive(Default)]
        struct FuzzInterface {
            address addr;
            string[] artifacts;
        }

        #[derive(Default)]
        function excludeArtifacts() public view returns (string[] memory excludedArtifacts);

        #[derive(Default)]
        function excludeContracts() public view returns (address[] memory excludedContracts);

        #[derive(Default)]
        function excludeSenders() public view returns (address[] memory excludedSenders);

        #[derive(Default)]
        function targetArtifacts() public view returns (string[] memory targetedArtifacts);

        #[derive(Default)]
        function targetArtifactSelectors() public view returns (FuzzAbiSelector[] memory targetedArtifactSelectors);

        #[derive(Default)]
        function targetContracts() public view returns (address[] memory targetedContracts);

        #[derive(Default)]
        function targetSelectors() public view returns (FuzzSelector[] memory targetedSelectors);

        #[derive(Default)]
        function targetSenders() public view returns (address[] memory targetedSenders);

        #[derive(Default)]
        function targetInterfaces() public view returns (FuzzInterface[] memory targetedInterfaces);
    }
}

/// Alias for (Dictionary for fuzzing, initial contracts to fuzz and an InvariantStrategy).
type InvariantPreparation = (
    EvmFuzzState,
    FuzzRunIdentifiedContracts,
    BoxedStrategy<BasicTxDetails>,
    CalldataFuzzDictionary,
);

/// Enriched results of an invariant run check.
///
/// Contains the success condition and call results of the last run
struct RichInvariantResults {
    success: bool,
    call_result: Option<RawCallResult>,
}

impl RichInvariantResults {
    fn new(success: bool, call_result: Option<RawCallResult>) -> Self {
        Self { success, call_result }
    }
}

/// Wrapper around any [`Executor`] implementor which provides fuzzing support using [`proptest`].
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contracts with
/// inputs, until it finds a counterexample sequence. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](proptest::test_runner::Config)
pub struct InvariantExecutor<'a> {
    pub executor: Executor,
    /// Proptest runner.
    runner: TestRunner,
    /// The invariant configuration
    config: InvariantConfig,
    /// Contracts deployed with `setUp()`
    setup_contracts: &'a ContractsByAddress,
    /// Contracts that are part of the project but have not been deployed yet. We need the bytecode
    /// to identify them from the stateset changes.
    project_contracts: &'a ContractsByArtifact,
    /// Filters contracts to be fuzzed through their artifact identifiers.
    artifact_filters: ArtifactFilters,
}

impl<'a> InvariantExecutor<'a> {
    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        executor: Executor,
        runner: TestRunner,
        config: InvariantConfig,
        setup_contracts: &'a ContractsByAddress,
        project_contracts: &'a ContractsByArtifact,
    ) -> Self {
        Self {
            executor,
            runner,
            config,
            setup_contracts,
            project_contracts,
            artifact_filters: ArtifactFilters::default(),
        }
    }

    /// Fuzzes any deployed contract and checks any broken invariant at `invariant_address`.
    pub fn invariant_fuzz(
        &mut self,
        invariant_contract: InvariantContract<'_>,
    ) -> Result<InvariantFuzzTestResult> {
        // Throw an error to abort test run if the invariant function accepts input params
        if !invariant_contract.invariant_function.inputs.is_empty() {
            return Err(eyre!("Invariant test function should have no inputs"))
        }

        let (fuzz_state, targeted_contracts, strat, calldata_fuzz_dictionary) =
            self.prepare_fuzzing(&invariant_contract)?;

        // Stores the consumed gas and calldata of every successful fuzz call.
        let fuzz_cases: RefCell<Vec<FuzzedCases>> = RefCell::new(Default::default());

        // Stores data related to reverts or failed assertions of the test.
        let failures = RefCell::new(InvariantFailures::new());

        // Stores the calldata in the last run.
        let last_run_calldata: RefCell<Vec<BasicTxDetails>> = RefCell::new(vec![]);

        // Stores additional traces for gas report.
        let gas_report_traces: RefCell<Vec<Vec<CallTraceArena>>> = RefCell::default();

        // Let's make sure the invariant is sound before actually starting the run:
        // We'll assert the invariant in its initial state, and if it fails, we'll
        // already know if we can early exit the invariant run.
        // This does not count as a fuzz run. It will just register the revert.
        let last_call_results = RefCell::new(assert_invariants(
            &invariant_contract,
            &targeted_contracts,
            &self.executor,
            &[],
            &mut failures.borrow_mut(),
            self.config.shrink_sequence,
            self.config.shrink_run_limit,
        )?);

        if last_call_results.borrow().is_none() {
            fuzz_cases.borrow_mut().push(FuzzedCases::new(vec![]));
        }

        // The strategy only comes with the first `input`. We fill the rest of the `inputs`
        // until the desired `depth` so we can use the evolving fuzz dictionary
        // during the run. We need another proptest runner to query for random
        // values.
        let branch_runner = RefCell::new(self.runner.clone());
        let _ = self.runner.run(&strat, |first_input| {
            let mut inputs = vec![first_input];

            // We stop the run immediately if we have reverted, and `fail_on_revert` is set.
            if self.config.fail_on_revert && failures.borrow().reverts > 0 {
                return Err(TestCaseError::fail("Revert occurred."))
            }

            // Before each run, we must reset the backend state.
            let mut executor = self.executor.clone();

            // Used for stat reports (eg. gas usage).
            let mut fuzz_runs = Vec::with_capacity(self.config.depth as usize);

            // Created contracts during a run.
            let mut created_contracts = vec![];

            // Traces of each call of the sequence.
            let mut run_traces = Vec::new();

            let mut current_run = 0;
            let mut assume_rejects_counter = 0;

            while current_run < self.config.depth {
                let (sender, (address, calldata)) = inputs.last().expect("no input generated");

                // Executes the call from the randomly generated sequence.
                let call_result = if self.config.preserve_state {
                    executor
                        .call_raw_committing(*sender, *address, calldata.clone(), U256::ZERO)
                        .expect("could not make raw evm call")
                } else {
                    executor
                        .call_raw(*sender, *address, calldata.clone(), U256::ZERO)
                        .expect("could not make raw evm call")
                };

                if call_result.result.as_ref() == MAGIC_ASSUME {
                    inputs.pop();
                    assume_rejects_counter += 1;
                    if assume_rejects_counter > self.config.max_assume_rejects {
                        failures.borrow_mut().error = Some(InvariantFuzzError::MaxAssumeRejects(
                            self.config.max_assume_rejects,
                        ));
                        return Err(TestCaseError::fail("Max number of vm.assume rejects reached."))
                    }
                } else {
                    // Collect data for fuzzing from the state changeset.
                    let mut state_changeset =
                        call_result.state_changeset.to_owned().expect("no changesets");

                    collect_data(&mut state_changeset, sender, &call_result, &fuzz_state);

                    // Collect created contracts and add to fuzz targets only if targeted contracts
                    // are updatable.
                    if targeted_contracts.is_updatable {
                        if let Err(error) = collect_created_contracts(
                            &state_changeset,
                            self.project_contracts,
                            self.setup_contracts,
                            &self.artifact_filters,
                            &targeted_contracts,
                            &mut created_contracts,
                        ) {
                            warn!(target: "forge::test", "{error}");
                        }
                    }
                    // Commit changes to the database.
                    executor.backend.commit(state_changeset.clone());

                    fuzz_runs.push(FuzzCase {
                        calldata: calldata.clone(),
                        gas: call_result.gas_used,
                        stipend: call_result.stipend,
                    });

                    let RichInvariantResults { success: can_continue, call_result: call_results } =
                        can_continue(
                            &invariant_contract,
                            call_result,
                            &executor,
                            &inputs,
                            &mut failures.borrow_mut(),
                            &targeted_contracts,
                            &state_changeset,
                            self.config.fail_on_revert,
                            self.config.shrink_sequence,
                            self.config.shrink_run_limit,
                            &mut run_traces,
                        )
                        .map_err(|e| TestCaseError::fail(e.to_string()))?;

                    if !can_continue || current_run == self.config.depth - 1 {
                        last_run_calldata.borrow_mut().clone_from(&inputs);
                    }

                    if !can_continue {
                        break
                    }

                    *last_call_results.borrow_mut() = call_results;
                    current_run += 1;
                }

                // Generates the next call from the run using the recently updated
                // dictionary.
                inputs.push(
                    strat
                        .new_tree(&mut branch_runner.borrow_mut())
                        .map_err(|_| TestCaseError::Fail("Could not generate case".into()))?
                        .current(),
                );
            }

            // We clear all the targeted contracts created during this run.
            if !created_contracts.is_empty() {
                let mut writable_targeted = targeted_contracts.targets.lock();
                for addr in created_contracts.iter() {
                    writable_targeted.remove(addr);
                }
            }

            if gas_report_traces.borrow().len() < self.config.gas_report_samples as usize {
                gas_report_traces.borrow_mut().push(run_traces);
            }
            fuzz_cases.borrow_mut().push(FuzzedCases::new(fuzz_runs));

            // Revert state to not persist values between runs.
            fuzz_state.revert();

            Ok(())
        });

        trace!(target: "forge::test::invariant::calldata_address_fuzz_dictionary", "{:?}", calldata_fuzz_dictionary.inner.addresses);
        trace!(target: "forge::test::invariant::dictionary", "{:?}", fuzz_state.dictionary_read().values().iter().map(hex::encode).collect::<Vec<_>>());

        let (reverts, error) = failures.into_inner().into_inner();

        Ok(InvariantFuzzTestResult {
            error,
            cases: fuzz_cases.into_inner(),
            reverts,
            last_run_inputs: last_run_calldata.take(),
            gas_report_traces: gas_report_traces.into_inner(),
        })
    }

    /// Prepares certain structures to execute the invariant tests:
    /// * Fuzz dictionary
    /// * Targeted contracts
    /// * Invariant Strategy
    fn prepare_fuzzing(
        &mut self,
        invariant_contract: &InvariantContract<'_>,
    ) -> eyre::Result<InvariantPreparation> {
        // Finds out the chosen deployed contracts and/or senders.
        self.select_contract_artifacts(invariant_contract.address)?;
        let (targeted_senders, targeted_contracts) =
            self.select_contracts_and_senders(invariant_contract.address)?;

        // Stores fuzz state for use with [fuzz_calldata_from_state].
        let fuzz_state: EvmFuzzState =
            build_initial_state(self.executor.backend.mem_db(), self.config.dictionary);

        let calldata_fuzz_config =
            CalldataFuzzDictionary::new(&self.config.dictionary, &fuzz_state);

        // Creates the invariant strategy.
        let strat = invariant_strat(
            fuzz_state.clone(),
            targeted_senders,
            targeted_contracts.clone(),
            self.config.dictionary.dictionary_weight,
            calldata_fuzz_config.clone(),
        )
        .no_shrink()
        .boxed();

        // Allows `override_call_strat` to use the address given by the Fuzzer inspector during
        // EVM execution.
        let mut call_generator = None;
        if self.config.call_override {
            let target_contract_ref = Arc::new(RwLock::new(Address::ZERO));

            call_generator = Some(RandomCallGenerator::new(
                invariant_contract.address,
                self.runner.clone(),
                override_call_strat(
                    fuzz_state.clone(),
                    targeted_contracts.clone(),
                    target_contract_ref.clone(),
                    calldata_fuzz_config.clone(),
                ),
                target_contract_ref,
            ));
        }

        self.executor.inspector.fuzzer =
            Some(Fuzzer { call_generator, fuzz_state: fuzz_state.clone(), collect: true });

        Ok((fuzz_state, targeted_contracts, strat, calldata_fuzz_config))
    }

    /// Fills the `InvariantExecutor` with the artifact identifier filters (in `path:name` string
    /// format). They will be used to filter contracts after the `setUp`, and more importantly,
    /// during the runs.
    ///
    /// Also excludes any contract without any mutable functions.
    ///
    /// Priority:
    ///
    /// targetArtifactSelectors > excludeArtifacts > targetArtifacts
    pub fn select_contract_artifacts(&mut self, invariant_address: Address) -> eyre::Result<()> {
        let result = self
            .call_sol_default(invariant_address, &IInvariantTest::targetArtifactSelectorsCall {});

        // Insert them into the executor `targeted_abi`.
        for IInvariantTest::FuzzAbiSelector { contract_abi, selectors } in
            result.targetedArtifactSelectors
        {
            let identifier = self.validate_selected_contract(contract_abi, &selectors)?;
            self.artifact_filters.targeted.entry(identifier).or_default().extend(selectors);
        }

        let selected =
            self.call_sol_default(invariant_address, &IInvariantTest::targetArtifactsCall {});
        let excluded =
            self.call_sol_default(invariant_address, &IInvariantTest::excludeArtifactsCall {});

        // Insert `excludeArtifacts` into the executor `excluded_abi`.
        for contract in excluded.excludedArtifacts {
            let identifier = self.validate_selected_contract(contract, &[])?;

            if !self.artifact_filters.excluded.contains(&identifier) {
                self.artifact_filters.excluded.push(identifier);
            }
        }

        // Exclude any artifact without mutable functions.
        for (artifact, contract) in self.project_contracts.iter() {
            if contract
                .abi
                .functions()
                .filter(|func| {
                    !matches!(
                        func.state_mutability,
                        alloy_json_abi::StateMutability::Pure |
                            alloy_json_abi::StateMutability::View
                    )
                })
                .count() ==
                0 &&
                !self.artifact_filters.excluded.contains(&artifact.identifier())
            {
                self.artifact_filters.excluded.push(artifact.identifier());
            }
        }

        // Insert `targetArtifacts` into the executor `targeted_abi`, if they have not been seen
        // before.
        for contract in selected.targetedArtifacts {
            let identifier = self.validate_selected_contract(contract, &[])?;

            if !self.artifact_filters.targeted.contains_key(&identifier) &&
                !self.artifact_filters.excluded.contains(&identifier)
            {
                self.artifact_filters.targeted.insert(identifier, vec![]);
            }
        }
        Ok(())
    }

    /// Makes sure that the contract exists in the project. If so, it returns its artifact
    /// identifier.
    fn validate_selected_contract(
        &mut self,
        contract: String,
        selectors: &[FixedBytes<4>],
    ) -> eyre::Result<String> {
        if let Some((artifact, contract_data)) =
            self.project_contracts.find_by_name_or_identifier(&contract)?
        {
            // Check that the selectors really exist for this contract.
            for selector in selectors {
                contract_data
                    .abi
                    .functions()
                    .find(|func| func.selector().as_slice() == selector.as_slice())
                    .wrap_err(format!("{contract} does not have the selector {selector:?}"))?;
            }

            return Ok(artifact.identifier())
        }
        eyre::bail!("{contract} not found in the project. Allowed format: `contract_name` or `contract_path:contract_name`.");
    }

    /// Selects senders and contracts based on the contract methods `targetSenders() -> address[]`,
    /// `targetContracts() -> address[]` and `excludeContracts() -> address[]`.
    pub fn select_contracts_and_senders(
        &self,
        to: Address,
    ) -> eyre::Result<(SenderFilters, FuzzRunIdentifiedContracts)> {
        let targeted_senders =
            self.call_sol_default(to, &IInvariantTest::targetSendersCall {}).targetedSenders;
        let excluded_senders =
            self.call_sol_default(to, &IInvariantTest::excludeSendersCall {}).excludedSenders;
        let selected =
            self.call_sol_default(to, &IInvariantTest::targetContractsCall {}).targetedContracts;
        let excluded =
            self.call_sol_default(to, &IInvariantTest::excludeContractsCall {}).excludedContracts;

        let mut contracts: TargetedContracts = self
            .setup_contracts
            .clone()
            .into_iter()
            .filter(|(addr, (identifier, _))| {
                *addr != to &&
                    *addr != CHEATCODE_ADDRESS &&
                    *addr != HARDHAT_CONSOLE_ADDRESS &&
                    (selected.is_empty() || selected.contains(addr)) &&
                    (self.artifact_filters.targeted.is_empty() ||
                        self.artifact_filters.targeted.contains_key(identifier)) &&
                    (excluded.is_empty() || !excluded.contains(addr)) &&
                    (self.artifact_filters.excluded.is_empty() ||
                        !self.artifact_filters.excluded.contains(identifier))
            })
            .map(|(addr, (identifier, abi))| (addr, (identifier, abi, vec![])))
            .collect();

        self.target_interfaces(to, &mut contracts)?;

        self.select_selectors(to, &mut contracts)?;

        // There should be at least one contract identified as target for fuzz runs.
        if contracts.is_empty() {
            eyre::bail!("No contracts to fuzz.");
        }

        Ok((
            SenderFilters::new(targeted_senders, excluded_senders),
            FuzzRunIdentifiedContracts::new(contracts, selected.is_empty()),
        ))
    }

    /// Extends the contracts and selectors to fuzz with the addresses and ABIs specified in
    /// `targetInterfaces() -> (address, string[])[]`. Enables targeting of addresses that are
    /// not deployed during `setUp` such as when fuzzing in a forked environment. Also enables
    /// targeting of delegate proxies and contracts deployed with `create` or `create2`.
    pub fn target_interfaces(
        &self,
        invariant_address: Address,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        let interfaces = self
            .call_sol_default(invariant_address, &IInvariantTest::targetInterfacesCall {})
            .targetedInterfaces;

        // Since `targetInterfaces` returns a tuple array there is no guarantee
        // that the addresses are unique this map is used to merge functions of
        // the specified interfaces for the same address. For example:
        // `[(addr1, ["IERC20", "IOwnable"])]` and `[(addr1, ["IERC20"]), (addr1, ("IOwnable"))]`
        // should be equivalent.
        let mut combined: TargetedContracts = BTreeMap::new();

        // Loop through each address and its associated artifact identifiers.
        // We're borrowing here to avoid taking full ownership.
        for IInvariantTest::FuzzInterface { addr, artifacts } in &interfaces {
            // Identifiers are specified as an array, so we loop through them.
            for identifier in artifacts {
                // Try to find the contract by name or identifier in the project's contracts.
                if let Some((_, contract)) =
                    self.project_contracts.find_by_name_or_identifier(identifier)?
                {
                    combined
                        // Check if there's an entry for the given key in the 'combined' map.
                        .entry(*addr)
                        // If the entry exists, extends its ABI with the function list.
                        .and_modify(|entry| {
                            let (_, contract_abi, _) = entry;

                            // Extend the ABI's function list with the new functions.
                            contract_abi.functions.extend(contract.abi.functions.clone());
                        })
                        // Otherwise insert it into the map.
                        .or_insert_with(|| (identifier.to_string(), contract.abi.clone(), vec![]));
                }
            }
        }

        targeted_contracts.extend(combined);

        Ok(())
    }

    /// Selects the functions to fuzz based on the contract method `targetSelectors()` and
    /// `targetArtifactSelectors()`.
    pub fn select_selectors(
        &self,
        address: Address,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        let some_abi_selectors = self
            .artifact_filters
            .targeted
            .iter()
            .filter(|(_, selectors)| !selectors.is_empty())
            .collect::<BTreeMap<_, _>>();

        for (address, (identifier, _)) in self.setup_contracts.iter() {
            if let Some(selectors) = some_abi_selectors.get(identifier) {
                self.add_address_with_functions(
                    *address,
                    (*selectors).clone(),
                    targeted_contracts,
                )?;
            }
        }

        let selectors = self.call_sol_default(address, &IInvariantTest::targetSelectorsCall {});
        for IInvariantTest::FuzzSelector { addr, selectors } in selectors.targetedSelectors {
            self.add_address_with_functions(addr, selectors, targeted_contracts)?;
        }
        Ok(())
    }

    /// Adds the address and fuzzable functions to `TargetedContracts`.
    fn add_address_with_functions(
        &self,
        address: Address,
        bytes4_array: Vec<FixedBytes<4>>,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        if let Some((name, abi, address_selectors)) = targeted_contracts.get_mut(&address) {
            // The contract is already part of our filter, and all we do is specify that we're
            // only looking at specific functions coming from `bytes4_array`.
            for selector in bytes4_array {
                address_selectors.push(get_function(name, &selector, abi)?);
            }
        } else {
            let (name, abi) = self.setup_contracts.get(&address).ok_or_else(|| {
                eyre::eyre!(
                    "[targetSelectors] address does not have an associated contract: {address}"
                )
            })?;

            let functions = bytes4_array
                .into_iter()
                .map(|selector| get_function(name, &selector, abi))
                .collect::<Result<Vec<_>, _>>()?;

            targeted_contracts.insert(address, (name.to_string(), abi.clone(), functions));
        }
        Ok(())
    }

    fn call_sol_default<C: SolCall>(&self, to: Address, args: &C) -> C::Return
    where
        C::Return: Default,
    {
        self.executor
            .call_sol(CALLER, to, args, U256::ZERO, None)
            .map(|c| c.decoded_result)
            .inspect_err(|e| warn!(target: "forge::test", "failed calling {:?}: {e}", C::SIGNATURE))
            .unwrap_or_default()
    }
}

/// Collects data from call for fuzzing. However, it first verifies that the sender is not an EOA
/// before inserting it into the dictionary. Otherwise, we flood the dictionary with
/// randomly generated addresses.
fn collect_data(
    state_changeset: &mut HashMap<Address, revm::primitives::Account>,
    sender: &Address,
    call_result: &RawCallResult,
    fuzz_state: &EvmFuzzState,
) {
    // Verify it has no code.
    let mut has_code = false;
    if let Some(Some(code)) = state_changeset.get(sender).map(|account| account.info.code.as_ref())
    {
        has_code = !code.is_empty();
    }

    // We keep the nonce changes to apply later.
    let mut sender_changeset = None;
    if !has_code {
        sender_changeset = state_changeset.remove(sender);
    }

    fuzz_state.collect_state_from_call(&call_result.logs, &*state_changeset);

    // Re-add changes
    if let Some(changed) = sender_changeset {
        state_changeset.insert(*sender, changed);
    }
}

/// Verifies that the invariant run execution can continue.
/// Returns the mapping of (Invariant Function Name -> Call Result, Logs, Traces) if invariants were
/// asserted.
#[allow(clippy::too_many_arguments)]
fn can_continue(
    invariant_contract: &InvariantContract<'_>,
    call_result: RawCallResult,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    failures: &mut InvariantFailures,
    targeted_contracts: &FuzzRunIdentifiedContracts,
    state_changeset: &StateChangeset,
    fail_on_revert: bool,
    shrink_sequence: bool,
    shrink_run_limit: usize,
    run_traces: &mut Vec<CallTraceArena>,
) -> eyre::Result<RichInvariantResults> {
    let mut call_results = None;

    // Detect handler assertion failures first.
    let handlers_failed = targeted_contracts.targets.lock().iter().any(|contract| {
        !executor.is_success(*contract.0, false, Cow::Borrowed(state_changeset), false)
    });

    // Assert invariants IF the call did not revert and the handlers did not fail.
    if !call_result.reverted && !handlers_failed {
        if let Some(traces) = call_result.traces {
            run_traces.push(traces);
        }

        call_results = assert_invariants(
            invariant_contract,
            targeted_contracts,
            executor,
            calldata,
            failures,
            shrink_sequence,
            shrink_run_limit,
        )?;
        if call_results.is_none() {
            return Ok(RichInvariantResults::new(false, None));
        }
    } else {
        // Increase the amount of reverts.
        failures.reverts += 1;
        // If fail on revert is set, we must return immediately.
        if fail_on_revert {
            let case_data = FailedInvariantCaseData::new(
                invariant_contract,
                targeted_contracts,
                None,
                calldata,
                call_result,
                &[],
                shrink_sequence,
                shrink_run_limit,
            );
            failures.revert_reason = Some(case_data.revert_reason.clone());
            let error = InvariantFuzzError::Revert(case_data);
            failures.error = Some(error);

            return Ok(RichInvariantResults::new(false, None));
        }
    }
    Ok(RichInvariantResults::new(true, call_results))
}
