//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::fuzz::{strategies::fuzz_param, *};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes, U256},
};
pub use proptest::test_runner::Config as FuzzConfig;
use proptest::{
    prelude::*,
    test_runner::{TestError, TestRunner},
};
use revm::db::DatabaseRef;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap};
use tracing::warn;

use crate::executor::{Executor, RawCallResult};

/// Wrapper around any [`Evm`](crate::Evm) implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided `TestRunner` contains all the
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

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case
    pub fn invariant_fuzz(
        &mut self,
        invariants: Vec<&Function>,
        invariant_address: Address,
        abi: &Abi,
        invariant_depth: u32,
    ) -> Option<InvariantFuzzTestResult> {
        let contracts = self.select_contracts(invariant_address, abi);
        let strat = invariant_strat(dbg!(invariant_depth as usize), contracts);

        // stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzCase>> = RefCell::new(Default::default());

        // stores the latest reason of a test call, this will hold the return reason of failed test
        // case if the runner failed
        let revert_reason = RefCell::new(None);
        let mut all_invars = BTreeMap::new();
        invariants.iter().for_each(|f| {
            all_invars.insert(f.name.to_string(), None);
        });
        let invariant_doesnt_hold = RefCell::new(all_invars);

        self.evm.set_tracing(false);
        let clean_db = self.evm.db.clone();
        let executor = RefCell::new(&mut self.evm);

        let _test_error = self
            .runner
            .run(&strat, |inputs| {
                'all: for (address, calldata) in inputs.iter() {
                    let RawCallResult { reverted, gas, stipend, .. } = executor
                        .borrow_mut()
                        .call_raw_committing(
                            self.sender,
                            *address,
                            calldata.0.clone(),
                            U256::zero(),
                        )
                        .expect("could not make raw evm call");

                    if !reverted {
                        // iterate over invariants, making sure they dont fail
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
                                    Some(InvariantFuzzError {
                                        test_error: proptest::test_runner::TestError::Fail(
                                            format!(
                                                "{}, reason: '{}'",
                                                func.name,
                                                match foundry_utils::decode_revert(
                                                    result.as_ref(),
                                                    Some(abi)
                                                ) {
                                                    Ok(e) => e,
                                                    Err(e) => e.to_string(),
                                                }
                                            )
                                            .into(),
                                            inputs.clone(),
                                        ),
                                        return_reason: "".into(),
                                        // return_reason: status,
                                        revert_reason: foundry_utils::decode_revert(
                                            result.as_ref(),
                                            Some(abi),
                                        )
                                        .unwrap_or_default(),
                                        addr: invariant_address,
                                        func: func.short_signature().into(),
                                    }),
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
                                        Some(InvariantFuzzError {
                                            test_error: proptest::test_runner::TestError::Fail(
                                                format!(
                                                    "{}, reason: '{}'",
                                                    func.name,
                                                    match foundry_utils::decode_revert(
                                                        result.as_ref(),
                                                        Some(abi)
                                                    ) {
                                                        Ok(e) => e,
                                                        Err(e) => e.to_string(),
                                                    }
                                                )
                                                .into(),
                                                inputs.clone(),
                                            ),
                                            return_reason: "".into(),
                                            revert_reason: foundry_utils::decode_revert(
                                                result.as_ref(),
                                                Some(abi),
                                            )
                                            .unwrap_or_default(),
                                            addr: invariant_address,
                                            func: func.short_signature().into(),
                                        }),
                                    );
                                    break 'all
                                }
                            }
                        }
                        // push test case to the case set
                        fuzz_cases.borrow_mut().push(FuzzCase {
                            calldata: calldata.clone(),
                            gas,
                            stipend,
                        });
                    } else {
                        // call failed, continue on
                    }
                }

                // Before each test, we must reset to the initial state
                executor.borrow_mut().db = clean_db.clone();

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
            cases: FuzzedCases::new(fuzz_cases.into_inner()),
        })
    }

    pub fn select_contracts(
        &self,
        invariant_address: Address,
        abi: &Abi,
    ) -> BTreeMap<Address, (String, Abi)> {
        let mut selected: Vec<Address> = vec![];
        if let Some(func) = abi.functions().into_iter().find(|func| func.name == "targetContracts")
        {
            if let Ok(call_result) = self.evm.call::<Vec<Address>, _, _>(
                self.sender,
                invariant_address,
                func.clone(),
                (),
                U256::zero(),
                Some(abi),
            ) {
                selected = call_result.result;
            } else {
                warn!("The function targetContracts was found but there was an error querying addresses.");
            }
        };

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
                    (selected.is_empty() || selected.contains(addr))
            })
            .collect()
    }
}

/// The outcome of an invariant fuzz test
pub struct InvariantFuzzTestResult {
    pub invariants: BTreeMap<String, Option<InvariantFuzzError>>,
    /// Every successful fuzz test case
    pub cases: FuzzedCases,
}

pub struct InvariantFuzzError {
    /// The proptest error occurred as a result of a test case
    pub test_error: TestError<Vec<(Address, Bytes)>>,
    /// The return reason of the offending call
    pub return_reason: Reason,
    /// The revert string of the offending call
    pub revert_reason: String,
    /// Address of the invariant asserter
    pub addr: Address,
    /// Function data for invariant check
    pub func: ethers::prelude::Bytes,
}

/// Container type for all successful test cases
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvariantFuzzedCases {
    cases: Vec<FuzzCase>,
}

impl InvariantFuzzedCases {
    pub fn new(mut cases: Vec<FuzzCase>) -> Self {
        cases.sort_by_key(|c| c.gas);
        Self { cases }
    }

    pub fn cases(&self) -> &[FuzzCase] {
        &self.cases
    }

    pub fn into_cases(self) -> Vec<FuzzCase> {
        self.cases
    }

    /// Returns the median gas of all test cases
    pub fn median_gas(&self) -> u64 {
        let mid = self.cases.len() / 2;
        self.cases.get(mid).map(|c| c.gas).unwrap_or_default()
    }

    /// Returns the average gas use of all test cases
    pub fn mean_gas(&self) -> u64 {
        if self.cases.is_empty() {
            return 0
        }

        (self.cases.iter().map(|c| c.gas as u128).sum::<u128>() / self.cases.len() as u128) as u64
    }

    pub fn highest(&self) -> Option<&FuzzCase> {
        self.cases.last()
    }

    pub fn lowest(&self) -> Option<&FuzzCase> {
        self.cases.first()
    }

    pub fn highest_gas(&self) -> u64 {
        self.highest().map(|c| c.gas).unwrap_or_default()
    }

    pub fn lowest_gas(&self) -> u64 {
        self.lowest().map(|c| c.gas).unwrap_or_default()
    }
}

/// Data of a single fuzz test case
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvariantFuzzCase {
    /// The calldata used for this fuzz test
    pub calldata: Bytes,
    // Consumed gas
    pub gas: u64,
}

pub fn invariant_strat(
    depth: usize,
    contracts: BTreeMap<Address, (String, Abi)>,
) -> BoxedStrategy<Vec<(Address, Bytes)>> {
    let iters = 1..depth + 1;
    proptest::collection::vec(gen_call(contracts), iters).boxed()
}

fn gen_call(contracts: BTreeMap<Address, (String, Abi)>) -> BoxedStrategy<(Address, Bytes)> {
    let random_contract = select_random_contract(contracts);
    random_contract
        .prop_flat_map(move |(contract, abi)| {
            let func = select_random_function(abi);
            func.prop_flat_map(move |func| fuzz_calldata(contract, func))
        })
        .boxed()
}

fn select_random_contract(
    contracts: BTreeMap<Address, (String, Abi)>,
) -> impl Strategy<Value = (Address, Abi)> {
    let selectors = any::<prop::sample::Selector>();
    selectors.prop_map(move |selector| {
        let res = selector.select(&contracts);
        (*res.0, res.1 .1.clone())
    })
}

fn select_random_function(abi: Abi) -> impl Strategy<Value = Function> {
    let selectors = any::<prop::sample::Selector>();
    let possible_funcs: Vec<ethers::abi::Function> = abi
        .functions()
        .filter(|func| {
            !matches!(
                func.state_mutability,
                ethers::abi::StateMutability::Pure | ethers::abi::StateMutability::View
            )
        })
        .cloned()
        .collect();
    selectors.prop_map(move |selector| {
        let func = selector.select(&possible_funcs);
        func.clone()
    })
}

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_calldata(addr: Address, func: Function) -> impl Strategy<Value = (Address, Bytes)> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    let strats = func.inputs.iter().map(|input| fuzz_param(&input.kind)).collect::<Vec<_>>();

    strats.prop_map(move |tokens| {
        tracing::trace!(input = ?tokens);
        (addr, func.encode_input(&tokens).unwrap().into())
    })
}
