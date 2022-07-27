use super::{
    assert_invariants, BasicTxDetails, FuzzRunIdentifiedContracts, InvariantExecutor,
    InvariantFuzzError, InvariantFuzzTestResult, InvariantTestOptions, RandomCallGenerator,
};
use crate::{
    executor::{inspector::Fuzzer, Executor, RawCallResult},
    fuzz::{
        strategies::{
            build_initial_state, collect_created_contracts, collect_state_from_call,
            invariant_strat, override_call_strat, EvmFuzzState,
        },
        FuzzCase, FuzzedCases,
    },
};
use ethers::{
    abi::{Abi, Address, Function},
    prelude::{ArtifactId, U256},
};
use parking_lot::RwLock;
use proptest::{
    strategy::{BoxedStrategy, Strategy, ValueTree},
    test_runner::{TestCaseError, TestRunner},
};
use revm::DatabaseCommit;
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

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
        invariants: Vec<&Function>,
        invariant_address: Address,
        test_contract_abi: &Abi,
        test_options: InvariantTestOptions,
    ) -> eyre::Result<Option<InvariantFuzzTestResult>> {
        let (fuzz_state, targeted_contracts, strat) =
            self.prepare_fuzzing(invariant_address, test_contract_abi, test_options)?;

        // Stores the consumed gas and calldata of every successful fuzz call.
        let fuzz_cases: RefCell<Vec<FuzzedCases>> = RefCell::new(Default::default());

        // Stores data related to reverts or failed assertions of the test.
        let failures = RefCell::new(InvariantFailures::new(&invariants));

        let blank_executor = RefCell::new(&mut *self.executor);

        // Make sure invariants are sound even before starting to fuzz
        if assert_invariants(
            test_contract_abi,
            &blank_executor.borrow(),
            invariant_address,
            &invariants,
            &[],
            &mut failures.borrow_mut(),
        )
        .is_err()
        {
            fuzz_cases.borrow_mut().push(FuzzedCases::new(vec![]));
        }

        if failures.borrow().broken_invariants_count < invariants.len() {
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

                    if failures.borrow().broken_invariants_count == invariants.len() {
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
                    let RawCallResult {
                        result, reverted, gas, stipend, state_changeset, logs, ..
                    } = executor
                        .call_raw(*sender, *address, calldata.0.clone(), U256::zero())
                        .expect("could not make raw evm call");

                    // Collect data for fuzzing from the state changeset.
                    let state_changeset =
                        state_changeset.to_owned().expect("to have a state changeset.");

                    collect_state_from_call(&logs, &state_changeset, fuzz_state.clone());
                    collect_created_contracts(
                        &state_changeset,
                        self.project_contracts,
                        self.setup_contracts,
                        targeted_contracts.clone(),
                        &mut created_contracts,
                    );

                    // Commit changes to the database.
                    executor.backend_mut().db.commit(state_changeset);

                    fuzz_runs.push(FuzzCase { calldata: calldata.clone(), gas, stipend });

                    if !reverted {
                        if assert_invariants(
                            test_contract_abi,
                            &executor,
                            invariant_address,
                            &invariants,
                            &inputs,
                            &mut failures.borrow_mut(),
                        )
                        .is_err()
                        {
                            break 'fuzz_run
                        }
                    } else {
                        failures.borrow_mut().reverts += 1;

                        // The user might want to stop all execution if a revert happens to
                        // better bound their testing space.
                        if test_options.fail_on_revert {
                            let error = InvariantFuzzError::new(
                                invariant_address,
                                None,
                                test_contract_abi,
                                &result,
                                &inputs,
                                &[],
                            );

                            failures.borrow_mut().revert_reason = Some(error.revert_reason.clone());

                            // Hacky to provide the full error to the user.
                            for invariant in invariants.iter() {
                                failures
                                    .borrow_mut()
                                    .failed_invariants
                                    .insert(invariant.name.clone(), Some(error.clone()));
                            }

                            break 'fuzz_run
                        }
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
                    let mut writable_targeted = targeted_contracts.write();
                    for addr in created_contracts.iter() {
                        writable_targeted.remove(addr);
                    }
                }

                fuzz_cases.borrow_mut().push(FuzzedCases::new(fuzz_runs));

                Ok(())
            });
        }

        // TODO: only saving one sequence case per invariant failure. Do we want more?
        let (reverts, invariants) = failures.into_inner().into_inner();

        Ok(Some(InvariantFuzzTestResult { invariants, cases: fuzz_cases.into_inner(), reverts }))
    }

    /// Prepares certain structures to execute the invariant tests:
    /// * Fuzz dictionary
    /// * Targeted contracts
    /// * Invariant Strategy
    fn prepare_fuzzing(
        &mut self,
        invariant_address: Address,
        test_contract_abi: &Abi,
        test_options: InvariantTestOptions,
    ) -> eyre::Result<(EvmFuzzState, FuzzRunIdentifiedContracts, BoxedStrategy<Vec<BasicTxDetails>>)>
    {
        // Finds out the chosen deployed contracts and/or senders.
        let (targeted_senders, targeted_contracts) =
            self.select_contracts_and_senders(invariant_address, test_contract_abi)?;

        if targeted_contracts.is_empty() {
            eyre::bail!("No contracts to fuzz.");
        }

        // Stores fuzz state for use with [fuzz_calldata_from_state].
        let fuzz_state: EvmFuzzState = build_initial_state(&self.executor.backend().db);

        // During execution, any newly created contract is added here and used through the rest of
        // the fuzz run.
        let targeted_contracts: FuzzRunIdentifiedContracts =
            Arc::new(RwLock::new(targeted_contracts));

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
                invariant_address,
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
}

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
