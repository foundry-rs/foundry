//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::fuzz::{strategies::invariant_strat, *};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes, U256},
};
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::test_runner::{TestError, TestRunner};
use revm::db::DatabaseRef;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};
use tracing::warn;

use crate::executor::{Executor, RawCallResult};

/// Wrapper around any [`Executor`] implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contracts with
/// inputs, until it finds a counterexample sequence. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
pub struct InvariantExecutor<'a, DB: DatabaseRef + Clone> {
    // evm: RefCell<&'a mut E>,
    /// The VM todo executor
    pub evm: &'a mut Executor<DB>,
    runner: TestRunner,
    sender: Address,
    contracts: &'a BTreeMap<Address, (String, Abi)>,
}

impl<'a, DB> InvariantExecutor<'a, DB>
where
    DB: DatabaseRef + Clone,
{
    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        evm: &'a mut Executor<DB>,
        runner: TestRunner,
        sender: Address,
        contracts: &'a BTreeMap<Address, (String, Abi)>,
    ) -> Self {
        Self { evm, runner, sender, contracts }
    }

    /// Fuzzes any deployed contract and checks any broken invariant at `invariant_address`
    /// Returns a list of all the consumed gas and calldata of every invariant fuzz case
    pub fn invariant_fuzz(
        &mut self,
        invariants: Vec<&Function>,
        invariant_address: Address,
        abi: &Abi,
        invariant_depth: u32,
    ) -> Option<InvariantFuzzTestResult> {
        // Finds out the chosen deployed contracts and/or senders.
        let (senders, contracts) = self.select_contracts_and_senders(invariant_address, abi);

        // Creates strategy
        let strat = invariant_strat(invariant_depth as usize, senders, contracts);

        // Stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzedCases>> = RefCell::new(Default::default());

        // stores the latest reason of a test call, this will hold the return reason of failed test
        // case if the runner failed
        let revert_reason = RefCell::new(None);
        let mut all_invars = BTreeMap::new();
        invariants.iter().for_each(|f| {
            all_invars.insert(f.name.to_string(), None);
        });
        let invariant_doesnt_hold = RefCell::new(all_invars);

        // Prepare executor
        self.evm.set_tracing(false);
        let clean_db = self.evm.db.clone();
        let executor = RefCell::new(&mut self.evm);
        let reverts = Cell::new(0);
        let _test_error = self
            .runner
            .run(&strat, |inputs| {
                let mut sequence = vec![];
                'all: for (sender, (address, calldata)) in inputs.iter() {
                    // Send nth randomly assigned sender + contract + input
                    let RawCallResult { reverted, gas, stipend, .. } = executor
                        .borrow_mut()
                        .call_raw_committing(*sender, *address, calldata.0.clone(), U256::zero())
                        .expect("could not make raw evm call");

                    if !reverted {
                        // Check if it breaks any of the listed invariants
                        for func in invariants.iter() {
                            let RawCallResult { reverted, state_changeset, result, .. } = executor
                                .borrow()
                                .call_raw(
                                    self.sender,
                                    invariant_address,
                                    func.encode_input(&[])?.into(),
                                    U256::zero(),
                                )
                                .expect("EVM error");
                            if reverted {
                                invariant_doesnt_hold.borrow_mut().insert(
                                    func.name.clone(),
                                    Some(InvariantFuzzError::new(
                                        invariant_address,
                                        func,
                                        abi,
                                        &result,
                                        &inputs,
                                    )),
                                );
                                break 'all
                            } else {
                                // This will panic and get caught by the executor
                                if !executor.borrow().is_success(
                                    invariant_address,
                                    reverted,
                                    state_changeset.expect("we should have a state changeset"),
                                    false,
                                ) {
                                    invariant_doesnt_hold.borrow_mut().insert(
                                        func.name.clone(),
                                        Some(InvariantFuzzError::new(
                                            invariant_address,
                                            func,
                                            abi,
                                            &result,
                                            &inputs,
                                        )),
                                    );
                                    break 'all
                                }
                            }
                        }
                        // push test case to the case set
                        sequence.push(FuzzCase { calldata: calldata.clone(), gas, stipend });
                    } else {
                        reverts.set(reverts.get() + 1);
                    }
                }

                // Before each test, we must reset to the initial state
                executor.borrow_mut().db = clean_db.clone();
                fuzz_cases.borrow_mut().push(FuzzedCases::new(sequence));

                Ok(())
            })
            .err()
            .map(|test_error| InvariantFuzzError {
                test_error,
                // return_reason: return_reason.into_inner().expect("Reason must be set"),
                return_reason: "".into(),
                revert_reason: revert_reason.into_inner().expect("Revert error string must be set"),
                addr: invariant_address,
                func: ethers::prelude::Bytes::default(),
            });

        Some(InvariantFuzzTestResult {
            invariants: invariant_doesnt_hold.into_inner(),
            cases: fuzz_cases.into_inner(),
            reverts: reverts.get(),
        })
    }

    fn select_contracts_and_senders(
        &self,
        invariant_address: Address,
        abi: &Abi,
    ) -> (Vec<Address>, BTreeMap<Address, (String, Abi)>) {
        let [senders, selected, excluded] =
            ["targetSenders", "targetContracts", "excludeContracts"]
                .map(|method| self.get_addresses(invariant_address, abi, method));

        (
            senders,
            self.contracts
                .clone()
                .into_iter()
                .filter(|(addr, _)| {
                    *addr != invariant_address &&
                        *addr !=
                            Address::from_slice(
                                &hex::decode("7109709ECfa91a80626fF3989D68f67F5b1DD12D").unwrap(),
                            ) &&
                        *addr !=
                            Address::from_slice(
                                &hex::decode("000000000000000000636F6e736F6c652e6c6f67").unwrap(),
                            ) &&
                        (selected.is_empty() || selected.contains(addr)) &&
                        (excluded.is_empty() || !excluded.contains(addr))
                })
                .collect(),
        )
    }

    fn get_addresses(&self, address: Address, abi: &Abi, method_name: &str) -> Vec<Address> {
        let mut addresses = vec![];

        if let Some(func) = abi.functions().into_iter().find(|func| func.name == method_name) {
            if let Ok(call_result) = self.evm.call::<Vec<Address>, _, _>(
                address,
                address,
                func.clone(),
                (),
                U256::zero(),
                Some(abi),
            ) {
                addresses = call_result.result;
            } else {
                warn!(
                    "The function {} was found but there was an error querying addresses.",
                    method_name
                );
            }
        };

        addresses
    }
}

/// The outcome of an invariant fuzz test
pub struct InvariantFuzzTestResult {
    pub invariants: BTreeMap<String, Option<InvariantFuzzError>>,
    /// Every successful fuzz test case
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls
    pub reverts: usize,
}

pub struct InvariantFuzzError {
    /// The proptest error occurred as a result of a test case
    pub test_error: TestError<Vec<(Address, (Address, Bytes))>>,
    /// The return reason of the offending call
    pub return_reason: Reason,
    /// The revert string of the offending call
    pub revert_reason: String,
    /// Address of the invariant asserter
    pub addr: Address,
    /// Function data for invariant check
    pub func: ethers::prelude::Bytes,
}

impl InvariantFuzzError {
    fn new(
        invariant_address: Address,
        func: &Function,
        abi: &Abi,
        result: &bytes::Bytes,
        inputs: &[(Address, (Address, Bytes))],
    ) -> Self {
        InvariantFuzzError {
            test_error: proptest::test_runner::TestError::Fail(
                format!(
                    "{}, reason: '{}'",
                    func.name,
                    match foundry_utils::decode_revert(result.as_ref(), Some(abi)) {
                        Ok(e) => e,
                        Err(e) => e.to_string(),
                    }
                )
                .into(),
                inputs.to_vec(),
            ),
            return_reason: "".into(),
            // return_reason: status,
            revert_reason: foundry_utils::decode_revert(result.as_ref(), Some(abi))
                .unwrap_or_default(),
            addr: invariant_address,
            func: func.short_signature().into(),
        }
    }
}
