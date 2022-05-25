//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::fuzz::{strategies::invariant_strat, *};
use ethers::{
    abi::{Abi, FixedBytes, Function},
    types::{Address, Bytes, U256},
};
use eyre::ContextCompat;
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::test_runner::{TestError, TestRunner};
use revm::{db::DatabaseRef, DatabaseCommit};
use std::{
    borrow::Borrow,
    cell::{Cell, RefCell, RefMut},
    collections::BTreeMap,
};
use tracing::warn;

use crate::executor::{Executor, RawCallResult};

pub type TargetedContracts = BTreeMap<Address, (String, Abi, Vec<Function>)>;

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
        fail_on_revert: bool,
    ) -> eyre::Result<Option<InvariantFuzzTestResult>> {
        // Finds out the chosen deployed contracts and/or senders.
        let (senders, contracts) = self.select_contracts_and_senders(invariant_address, abi)?;

        // Stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzedCases>> = RefCell::new(Default::default());

        // Stores fuzz state for use with [fuzz_calldata_from_state]
        let fuzz_state: EvmFuzzState = build_initial_state(&self.evm.db);

        // Creates strategy
        let strat =
            invariant_strat(fuzz_state.clone(), invariant_depth as usize, senders, contracts);

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
                if fail_on_revert && reverts.get() > 0 {
                    return Err(TestCaseError::fail("Revert occurred."))
                }

                let mut sequence = vec![];

                'all: for (index, (sender, (address, calldata))) in inputs.iter().enumerate() {
                    // Send nth randomly assigned sender + contract + input
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
                    executor.borrow_mut().db.commit(state_changeset);

                    sequence.push(FuzzCase { calldata: calldata.clone(), gas, stipend });

                    if !reverted {
                        if let Err((func, result)) = assert_invariants(
                            self.sender,
                            executor.borrow_mut(),
                            invariant_address,
                            &invariants,
                        ) {
                            invariant_doesnt_hold.borrow_mut().insert(
                                func.name.clone(),
                                Some(InvariantFuzzError::new(
                                    invariant_address,
                                    Some(func),
                                    abi,
                                    &result,
                                    inputs.split_at(index + 1).0,
                                )),
                            );

                            break 'all
                        }
                    } else {
                        reverts.set(reverts.get() + 1);
                        if fail_on_revert {
                            let error = InvariantFuzzError::new(
                                invariant_address,
                                None,
                                abi,
                                &result,
                                inputs.split_at(index + 1).0,
                            );
                            *revert_reason.borrow_mut() = Some(error.revert_reason.clone());

                            invariant_doesnt_hold
                                .borrow_mut()
                                .insert("Revert".to_string(), Some(error));
                            break 'all
                        }
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
                func: Some(ethers::prelude::Bytes::default()),
            });

        Ok(Some(InvariantFuzzTestResult {
            invariants: invariant_doesnt_hold.into_inner(),
            cases: fuzz_cases.into_inner(),
            reverts: reverts.get(),
        }))
    }

    fn select_contracts_and_senders(
        &self,
        invariant_address: Address,
        abi: &Abi,
    ) -> eyre::Result<(Vec<Address>, TargetedContracts)> {
        let [senders, selected, excluded] =
            ["targetSenders", "targetContracts", "excludeContracts"]
                .map(|method| self.get_addresses(invariant_address, abi, method));

        let mut contracts: TargetedContracts = self
            .contracts
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
            .map(|(addr, (name, abi))| (addr, (name, abi, vec![])))
            .collect();

        self.get_selectors(invariant_address, abi, &mut contracts)?;

        Ok((senders, contracts))
    }

    fn get_addresses(&self, address: Address, abi: &Abi, method_name: &str) -> Vec<Address> {
        let mut addresses = vec![];

        if let Some(func) = abi.functions().into_iter().find(|func| func.name == method_name) {
            if let Ok(call_result) = self.evm.call::<Vec<Address>, _, _>(
                self.sender,
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

    fn get_selectors(
        &self,
        address: Address,
        abi: &Abi,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        let mut selectors: Vec<(Address, Vec<FixedBytes>)> = vec![];

        if let Some(func) = abi.functions().into_iter().find(|func| func.name == "targetSelectors")
        {
            if let Ok(call_result) = self.evm.call::<Vec<(Address, Vec<FixedBytes>)>, _, _>(
                self.sender,
                address,
                func.clone(),
                (),
                U256::zero(),
                Some(abi),
            ) {
                selectors = call_result.result;
            } else {
                warn!(
                    "The function {} was found but there was an error querying addresses.",
                    "targetSelectors"
                );
            }
        };

        fn add_function(
            name: &str,
            selector: FixedBytes,
            abi: &Abi,
            funcs: &mut Vec<Function>,
        ) -> eyre::Result<()> {
            let func = abi
                .functions()
                .into_iter()
                .find(|func| func.short_signature().as_slice() == selector.as_slice())
                .wrap_err(format!("{name} does not have the selector {:?}", selector))?;

            funcs.push(func.clone());
            Ok(())
        }

        for (address, bytes4_array) in selectors.into_iter() {
            if let Some((name, abi, address_selectors)) = targeted_contracts.get_mut(&address) {
                for selector in bytes4_array {
                    add_function(name, selector, abi, address_selectors)?;
                }
            } else {
                let (name, abi) = self.contracts.get(&address).wrap_err(format!(
                    "[targetSelectors] address does not have an associated contract: {}",
                    address
                ))?;
                let mut functions = vec![];
                for selector in bytes4_array {
                    add_function(name, selector, abi, &mut functions)?;
                }

                targeted_contracts.insert(address, (name.to_string(), abi.clone(), functions));
            }
        }
        Ok(())
    }
}

fn assert_invariants<'a, DB>(
    sender: Address,
    executor: RefMut<&mut &mut Executor<DB>>,
    invariant_address: Address,
    invariants: &'a [&Function],
) -> Result<(), (&'a Function, bytes::Bytes)>
where
    DB: DatabaseRef,
{
    for func in invariants {
        let RawCallResult { reverted, state_changeset, result, .. } = executor
            .borrow()
            .call_raw(
                sender,
                invariant_address,
                func.encode_input(&[]).expect("invariant should have no inputs").into(),
                U256::zero(),
            )
            .expect("EVM error");
        if reverted {
            return Err((*func, result))
        } else {
            // This will panic and get caught by the executor
            if !executor.borrow().is_success(
                invariant_address,
                reverted,
                state_changeset.expect("we should have a state changeset"),
                false,
            ) {
                return Err((*func, result))
            }
        }
    }
    Ok(())
}

/// The outcome of an invariant fuzz test
#[derive(Debug)]
pub struct InvariantFuzzTestResult {
    pub invariants: BTreeMap<String, Option<InvariantFuzzError>>,
    /// Every successful fuzz test case
    pub cases: Vec<FuzzedCases>,
    /// Number of reverted fuzz calls
    pub reverts: usize,
}

#[derive(Debug)]
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
    pub func: Option<ethers::prelude::Bytes>,
}

impl InvariantFuzzError {
    fn new(
        invariant_address: Address,
        error_func: Option<&Function>,
        abi: &Abi,
        result: &bytes::Bytes,
        inputs: &[(Address, (Address, Bytes))],
    ) -> Self {
        let mut func = None;
        let origin: String;

        if let Some(f) = error_func {
            func = Some(f.short_signature().into());
            origin = f.name.clone();
        } else {
            origin = "Revert".to_string();
        }

        InvariantFuzzError {
            test_error: proptest::test_runner::TestError::Fail(
                format!(
                    "{}, reason: '{}'",
                    origin,
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
            func,
        }
    }
}
