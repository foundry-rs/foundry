use parking_lot::RwLock;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    sync::Arc,
};

use ethers::{
    abi::{Abi, Address, Function},
    prelude::{ArtifactId, U256},
};
use proptest::{
    strategy::{Strategy, ValueTree},
    test_runner::{TestCaseError, TestRunner},
};
use revm::DatabaseCommit;

use crate::{
    executor::{Executor, RawCallResult},
    fuzz::{
        strategies::{
            build_initial_state, collect_created_contracts, collect_state_from_call,
            invariant_strat, override_call_strat, EvmFuzzState,
        },
        FuzzCase, FuzzedCases,
    },
};

use super::{
    assert_invariants, FuzzRunIdentifiedContracts, InvariantExecutor, InvariantFuzzError,
    InvariantFuzzTestResult, InvariantTestOptions, RandomCallGenerator,
};

impl<'a> InvariantExecutor<'a> {
    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        evm: &'a mut Executor,
        runner: TestRunner,
        sender: Address,
        setup_contracts: &'a BTreeMap<Address, (String, Abi)>,
        project_contracts: &'a BTreeMap<ArtifactId, (Abi, Vec<u8>)>,
    ) -> Self {
        Self { evm, runner, sender, setup_contracts, project_contracts }
    }

    /// Fuzzes any deployed contract and checks any broken invariant at `invariant_address`
    /// Returns a list of all the consumed gas and calldata of every invariant fuzz case
    pub fn invariant_fuzz(
        &mut self,
        invariants: Vec<&Function>,
        invariant_address: Address,
        abi: &Abi,
        test_options: InvariantTestOptions,
    ) -> eyre::Result<Option<InvariantFuzzTestResult>> {
        // Finds out the chosen deployed contracts and/or senders.
        let (targeted_senders, targeted_contracts) =
            self.select_contracts_and_senders(invariant_address, abi)?;

        if targeted_contracts.is_empty() {
            eyre::bail!("No contracts to fuzz!");
        }

        // Stores the consumed gas and calldata of every successful fuzz call.
        let fuzz_cases: RefCell<Vec<FuzzedCases>> = RefCell::new(Default::default());

        // Stores fuzz state for use with [fuzz_calldata_from_state].
        let fuzz_state: EvmFuzzState = build_initial_state(&self.evm.backend().db);

        // During execution, any newly created contract is added here and used through the rest of
        // the fuzz run.
        let targeted_contracts: FuzzRunIdentifiedContracts =
            Arc::new(RwLock::new(targeted_contracts));

        // Creates invariant strategy.
        let strat =
            invariant_strat(fuzz_state.clone(), targeted_senders, targeted_contracts.clone())
                .no_shrink();

        // Stores the latest reason of a test call.  If the runner fails, it will hold the return
        // reason of the failed test
        let revert_reason = RefCell::new(None);

        let failed_invariants =
            RefCell::new(invariants.iter().map(|f| (f.name.to_string(), None)).collect());

        // Prepare executor
        self.evm.set_tracing(false);

        let mut call_generator = None;
        if test_options.call_override {
            // Allows `override_call_strat` to use the address given by the Fuzzer inspector during
            // EVM execution.
            let target_contract_ref = Arc::new(RwLock::new(Address::zero()));

            call_generator = Some(RandomCallGenerator::new(
                self.runner.clone(),
                override_call_strat(
                    fuzz_state.clone(),
                    targeted_contracts.clone(),
                    target_contract_ref.clone(),
                ),
                target_contract_ref,
            ));
        }
        self.evm.set_fuzzer(call_generator, fuzz_state.clone());

        let clean_db = self.evm.backend().db.clone();
        let executor = RefCell::new(&mut self.evm);

        let reverts = Cell::new(0);
        let broken_invariants = Cell::new(0);

        // If a new contract is created, we need another runner to generate new inputs
        let branch_runner = RefCell::new(self.runner.clone());

        // The strategy only comes with the first `input`. We fill the rest of the `inputs` until
        // the desired `depth` so we can use the state during the run.
        let _test_error = self
            .runner
            .run(&strat, |mut inputs| {
                {
                    // Scenarios where we want to fail as soon as possible.
                    if test_options.fail_on_revert && reverts.get() == 1 {
                        return Err(TestCaseError::fail("Revert occurred."))
                    }

                    if broken_invariants.get() == invariants.len() {
                        return Err(TestCaseError::fail("All invariants have been broken."))
                    }
                }

                let mut current_depth = 0;
                let mut sequence = vec![];
                let mut created = vec![];

                'outer: while current_depth < test_options.depth {
                    let (sender, (address, calldata)) = inputs.last().unwrap();

                    // Executes the call from the randomly generated sequence
                    let RawCallResult {
                        result, reverted, gas, stipend, state_changeset, logs, ..
                    } = executor
                        .borrow()
                        .call_raw(*sender, *address, calldata.0.clone(), U256::zero())
                        .expect("could not make raw evm call");

                    // Collect data for fuzzing
                    let state_changeset =
                        state_changeset.to_owned().expect("we should have a state changeset");
                    collect_state_from_call(&logs, &state_changeset, fuzz_state.clone());

                    // Commit changes
                    executor.borrow_mut().backend_mut().db.commit(state_changeset.clone());
                    sequence.push(FuzzCase { calldata: calldata.clone(), gas, stipend });

                    if !reverted {
                        if assert_invariants(
                            self.sender,
                            abi,
                            &executor,
                            invariant_address,
                            &invariants,
                            failed_invariants.borrow_mut(),
                            &inputs,
                        )
                        .is_err()
                        {
                            broken_invariants.set(
                                failed_invariants
                                    .borrow()
                                    .iter()
                                    .filter_map(|case| case.1.as_ref())
                                    .count(),
                            );
                            break 'outer
                        }
                    } else {
                        reverts.set(reverts.get() + 1);
                        if test_options.fail_on_revert {
                            let error = InvariantFuzzError::new(
                                invariant_address,
                                None,
                                abi,
                                &result,
                                &inputs,
                                &[],
                            );
                            *revert_reason.borrow_mut() = Some(error.revert_reason.clone());

                            for invariant in invariants.iter() {
                                failed_invariants
                                    .borrow_mut()
                                    .insert(invariant.name.clone(), Some(error.clone()));
                            }

                            break 'outer
                        }
                    }

                    collect_created_contracts(
                        state_changeset,
                        self.project_contracts,
                        self.setup_contracts,
                        targeted_contracts.clone(),
                        &mut created,
                    );

                    // Generates the next call with an updated `EvmFuzzState`
                    inputs.extend(
                        strat
                            .new_tree(&mut branch_runner.borrow_mut())
                            .map_err(|_| TestCaseError::Fail("Could not generate case".into()))?
                            .current(),
                    );

                    current_depth += 1;
                }

                // Before each test, we must reset to the initial state
                executor.borrow_mut().backend_mut().db = clean_db.clone();

                // We clear all the targeted contracts created during this run
                if !created.is_empty() {
                    let mut targeted = targeted_contracts.write();
                    for addr in created.iter() {
                        targeted.remove(addr);
                    }
                }

                fuzz_cases.borrow_mut().push(FuzzedCases::new(sequence));

                Ok(())
            })
            .err()
            .map(|test_error| InvariantFuzzError {
                test_error,
                // return_reason: return_reason.into_inner().expect("Reason must be set"),
                return_reason: "".into(),
                revert_reason: "".into(), /* revert_reason.into_inner().expect("Revert error
                                           * string must be set"), */
                addr: invariant_address,
                func: Some(ethers::prelude::Bytes::default()),
                inner_sequence: vec![],
            });

        // TODO only saving one sequence case per invariant failure. Do we want more?
        Ok(Some(InvariantFuzzTestResult {
            invariants: failed_invariants.into_inner(),
            cases: fuzz_cases.into_inner(),
            reverts: reverts.get(),
        }))
    }
}
