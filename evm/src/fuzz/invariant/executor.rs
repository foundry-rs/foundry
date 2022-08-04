use super::{
    assert_invariants, BasicTxDetails, FuzzRunIdentifiedContracts, InvariantContract,
    InvariantFuzzError, InvariantFuzzTestResult, InvariantTestOptions, RandomCallGenerator,
    TargetedContracts,
};
use crate::{
    executor::{
        inspector::Fuzzer, Executor, RawCallResult, CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS,
    },
    fuzz::{
        strategies::{
            build_initial_state, collect_created_contracts, collect_state_from_call,
            invariant_strat, override_call_strat, EvmFuzzState,
        },
        FuzzCase, FuzzedCases,
    },
    CALLER,
};
use ethers::{
    abi::{Abi, Address, Detokenize, FixedBytes, Function, Tokenizable, TokenizableItem},
    prelude::{ArtifactId, U256},
};
use eyre::ContextCompat;
use parking_lot::{Mutex, RwLock};
use proptest::{
    strategy::{BoxedStrategy, Strategy, ValueTree},
    test_runner::{TestCaseError, TestRunner},
};
use revm::DatabaseCommit;
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use tracing::warn;

/// Alias for (Dictionary for fuzzing, initial contracts to fuzz and an InvariantStrategy).
type InvariantPreparation =
    (EvmFuzzState, FuzzRunIdentifiedContracts, BoxedStrategy<Vec<BasicTxDetails>>);

/// Wrapper around any [`Executor`] implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contracts with
/// inputs, until it finds a counterexample sequence. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
pub struct InvariantExecutor<'a> {
    pub executor: &'a mut Executor,
    /// Proptest runner.
    runner: TestRunner,
    /// Contracts deployed with `setUp()`
    setup_contracts: &'a BTreeMap<Address, (String, Abi)>,
    /// Contracts that are part of the project but have not been deployed yet. We need the bytecode
    /// to identify them from the stateset changes.
    project_contracts: &'a BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
}

impl<'a> InvariantExecutor<'a> {
    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        executor: &'a mut Executor,
        runner: TestRunner,
        setup_contracts: &'a BTreeMap<Address, (String, Abi)>,
        project_contracts: &'a BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    ) -> Self {
        Self { executor, runner, setup_contracts, project_contracts }
    }

    /// Fuzzes any deployed contract and checks any broken invariant at `invariant_address`
    /// Returns a list of all the consumed gas and calldata of every invariant fuzz case
    pub fn invariant_fuzz(
        &mut self,
        invariant_contract: InvariantContract,
        test_options: InvariantTestOptions,
    ) -> eyre::Result<Option<InvariantFuzzTestResult>> {
        let (fuzz_state, targeted_contracts, strat) =
            self.prepare_fuzzing(&invariant_contract, test_options)?;

        // Stores the consumed gas and calldata of every successful fuzz call.
        let fuzz_cases: RefCell<Vec<FuzzedCases>> = RefCell::new(Default::default());

        // Stores data related to reverts or failed assertions of the test.
        let failures =
            RefCell::new(InvariantFailures::new(&invariant_contract.invariant_functions));

        let blank_executor = RefCell::new(&mut *self.executor);

        // Make sure invariants are sound even before starting to fuzz
        if assert_invariants(
            &invariant_contract,
            &blank_executor.borrow(),
            &[],
            &mut failures.borrow_mut(),
        )
        .is_err()
        {
            fuzz_cases.borrow_mut().push(FuzzedCases::new(vec![]));
        }

        if failures.borrow().broken_invariants_count < invariant_contract.invariant_functions.len()
        {
            // The strategy only comes with the first `input`. We fill the rest of the `inputs`
            // until the desired `depth` so we can use the evolving fuzz dictionary
            // during the run. We need another proptest runner to query for random
            // values.
            let branch_runner = RefCell::new(self.runner.clone());
            let _ = self.runner.run(&strat, |mut inputs| {
                // Scenarios where we want to fail as soon as possible.
                {
                    if test_options.fail_on_revert && failures.borrow().reverts == 1 {
                        return Err(TestCaseError::fail("Revert occurred."))
                    }

                    if failures.borrow().broken_invariants_count ==
                        invariant_contract.invariant_functions.len()
                    {
                        return Err(TestCaseError::fail("All invariants have been broken."))
                    }
                }

                // Before each run, we must reset the backend state.
                let mut executor = blank_executor.borrow().clone();

                // Used for stat reports (eg. gas usage).
                let mut fuzz_runs = vec![];

                // Created contracts during a run.
                let mut created_contracts = vec![];

                'fuzz_run: for _ in 0..test_options.depth {
                    let (sender, (address, calldata)) =
                        inputs.last().expect("to have the next randomly generated input.");

                    // Executes the call from the randomly generated sequence.
                    let call_result = executor
                        .call_raw(*sender, *address, calldata.0.clone(), U256::zero())
                        .expect("could not make raw evm call");

                    // Collect data for fuzzing from the state changeset.
                    let state_changeset =
                        call_result.state_changeset.to_owned().expect("to have a state changeset.");

                    collect_state_from_call(
                        &call_result.logs,
                        &state_changeset,
                        fuzz_state.clone(),
                    );
                    collect_created_contracts(
                        &state_changeset,
                        self.project_contracts,
                        self.setup_contracts,
                        targeted_contracts.clone(),
                        &mut created_contracts,
                    );

                    // Commit changes to the database.
                    executor.backend_mut().commit(state_changeset);

                    fuzz_runs.push(FuzzCase {
                        calldata: calldata.clone(),
                        gas: call_result.gas,
                        stipend: call_result.stipend,
                    });

                    if !can_continue(
                        &invariant_contract,
                        call_result,
                        &executor,
                        &inputs,
                        &mut failures.borrow_mut(),
                        test_options,
                    ) {
                        break 'fuzz_run
                    }

                    // Generates the next call from the run using the recently updated
                    // dictionary.
                    inputs.extend(
                        strat
                            .new_tree(&mut branch_runner.borrow_mut())
                            .map_err(|_| TestCaseError::Fail("Could not generate case".into()))?
                            .current(),
                    );
                }

                // We clear all the targeted contracts created during this run.
                if !created_contracts.is_empty() {
                    let mut writable_targeted = targeted_contracts.lock();
                    for addr in created_contracts.iter() {
                        writable_targeted.remove(addr);
                    }
                }

                fuzz_cases.borrow_mut().push(FuzzedCases::new(fuzz_runs));

                Ok(())
            });
        }

        let (reverts, invariants) = failures.into_inner().into_inner();

        Ok(Some(InvariantFuzzTestResult { invariants, cases: fuzz_cases.into_inner(), reverts }))
    }

    /// Prepares certain structures to execute the invariant tests:
    /// * Fuzz dictionary
    /// * Targeted contracts
    /// * Invariant Strategy
    fn prepare_fuzzing(
        &mut self,
        invariant_contract: &InvariantContract,
        test_options: InvariantTestOptions,
    ) -> eyre::Result<InvariantPreparation> {
        // Finds out the chosen deployed contracts and/or senders.
        let (targeted_senders, targeted_contracts) =
            self.select_contracts_and_senders(invariant_contract.address, invariant_contract.abi)?;

        if targeted_contracts.is_empty() {
            eyre::bail!("No contracts to fuzz.");
        }

        // Stores fuzz state for use with [fuzz_calldata_from_state].
        let fuzz_state: EvmFuzzState = build_initial_state(self.executor.backend().mem_db());

        // During execution, any newly created contract is added here and used through the rest of
        // the fuzz run.
        let targeted_contracts: FuzzRunIdentifiedContracts =
            Arc::new(Mutex::new(targeted_contracts));

        // Creates the invariant strategy.
        let strat =
            invariant_strat(fuzz_state.clone(), targeted_senders, targeted_contracts.clone())
                .no_shrink()
                .boxed();

        // Allows `override_call_strat` to use the address given by the Fuzzer inspector during
        // EVM execution.
        let mut call_generator = None;
        if test_options.call_override {
            let target_contract_ref = Arc::new(RwLock::new(Address::zero()));

            call_generator = Some(RandomCallGenerator::new(
                invariant_contract.address,
                self.runner.clone(),
                override_call_strat(
                    fuzz_state.clone(),
                    targeted_contracts.clone(),
                    target_contract_ref.clone(),
                ),
                target_contract_ref,
            ));
        }

        self.executor.inspector_config_mut().fuzzer =
            Some(Fuzzer { call_generator, fuzz_state: fuzz_state.clone(), collect: true });

        // Tracing should be off when running all runs. It will be turned on later for the failure
        // cases.
        self.executor.set_tracing(false);

        Ok((fuzz_state, targeted_contracts, strat))
    }

    /// Selects senders and contracts based on the contract methods `targetSenders() -> address[]`,
    /// `targetContracts() -> address[]` and `excludeContracts() -> address[]`.
    pub fn select_contracts_and_senders(
        &self,
        invariant_address: Address,
        abi: &Abi,
    ) -> eyre::Result<(Vec<Address>, TargetedContracts)> {
        let [senders, selected, excluded] =
            ["targetSenders", "targetContracts", "excludeContracts"]
                .map(|method| self.get_list::<Address>(invariant_address, abi, method));

        let mut contracts: TargetedContracts = self
            .setup_contracts
            .clone()
            .into_iter()
            .filter(|(addr, _)| {
                *addr != invariant_address &&
                    *addr != CHEATCODE_ADDRESS &&
                    *addr != HARDHAT_CONSOLE_ADDRESS &&
                    (selected.is_empty() || selected.contains(addr)) &&
                    (excluded.is_empty() || !excluded.contains(addr))
            })
            .map(|(addr, (name, abi))| (addr, (name, abi, vec![])))
            .collect();

        self.select_selectors(invariant_address, abi, &mut contracts)?;

        Ok((senders, contracts))
    }

    /// Selects the functions to fuzz based on the contract method `targetSelectors() -> (address,
    /// bytes4[])[]`.
    pub fn select_selectors(
        &self,
        address: Address,
        abi: &Abi,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        let selectors =
            self.get_list::<(Address, Vec<FixedBytes>)>(address, abi, "targetSelectors");

        fn get_function(name: &str, selector: FixedBytes, abi: &Abi) -> eyre::Result<Function> {
            abi.functions()
                .into_iter()
                .find(|func| func.short_signature().as_slice() == selector.as_slice())
                .cloned()
                .wrap_err(format!("{name} does not have the selector {:?}", selector))
        }

        for (address, bytes4_array) in selectors.into_iter() {
            if let Some((name, abi, address_selectors)) = targeted_contracts.get_mut(&address) {
                // The contract is already part of our filter, and all we do is specify that we're
                // only looking at specific functions coming from `bytes4_array`.
                for selector in bytes4_array {
                    address_selectors.push(get_function(name, selector, abi)?);
                }
            } else {
                let (name, abi) = self.setup_contracts.get(&address).wrap_err(format!(
                    "[targetSelectors] address does not have an associated contract: {}",
                    address
                ))?;

                let functions = bytes4_array
                    .into_iter()
                    .map(|selector| get_function(name, selector, abi))
                    .collect::<Result<Vec<_>, _>>()?;

                targeted_contracts.insert(address, (name.to_string(), abi.clone(), functions));
            }
        }
        Ok(())
    }

    /// Gets list of `T` by calling the contract `method_name` function.
    fn get_list<T>(&self, address: Address, abi: &Abi, method_name: &str) -> Vec<T>
    where
        T: Tokenizable + Detokenize + TokenizableItem,
    {
        if let Some(func) = abi.functions().into_iter().find(|func| func.name == method_name) {
            if let Ok(call_result) = self.executor.call::<Vec<T>, _, _>(
                CALLER,
                address,
                func.clone(),
                (),
                U256::zero(),
                Some(abi),
            ) {
                return call_result.result
            } else {
                warn!(
                    "The function {} was found but there was an error querying its data.",
                    method_name
                );
            }
        };

        Vec::new()
    }
}

/// Verifies that the invariant run execution can continue.
fn can_continue(
    invariant_contract: &InvariantContract,
    call_result: RawCallResult,
    executor: &Executor,
    calldata: &[BasicTxDetails],
    failures: &mut InvariantFailures,
    test_options: InvariantTestOptions,
) -> bool {
    if !call_result.reverted {
        if assert_invariants(invariant_contract, executor, calldata, failures).is_err() {
            return false
        }
    } else {
        failures.reverts += 1;

        // The user might want to stop all execution if a revert happens to
        // better bound their testing space.
        if test_options.fail_on_revert {
            let error =
                InvariantFuzzError::new(invariant_contract, None, calldata, call_result, &[]);

            failures.revert_reason = Some(error.revert_reason.clone());

            // Hacky to provide the full error to the user.
            for invariant in invariant_contract.invariant_functions.iter() {
                failures.failed_invariants.insert(invariant.name.clone(), Some(error.clone()));
            }

            return false
        }
    }
    true
}

#[derive(Clone)]
/// Stores information about failures and reverts of the invariant tests.
pub struct InvariantFailures {
    /// The latest revert reason of a run.
    pub revert_reason: Option<String>,
    /// Total number of reverts.
    pub reverts: usize,
    /// How many different invariants have been broken.
    pub broken_invariants_count: usize,
    /// Maps a broken invariant to its specific error.
    pub failed_invariants: BTreeMap<String, Option<InvariantFuzzError>>,
}

impl InvariantFailures {
    fn new(invariants: &[&Function]) -> Self {
        InvariantFailures {
            reverts: 0,
            broken_invariants_count: 0,
            failed_invariants: invariants.iter().map(|f| (f.name.to_string(), None)).collect(),
            revert_reason: None,
        }
    }

    /// Moves `reverts` and `failed_invariants` out of the struct.
    fn into_inner(self) -> (usize, BTreeMap<String, Option<InvariantFuzzError>>) {
        (self.reverts, self.failed_invariants)
    }
}
