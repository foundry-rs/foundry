//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used

use crate::{
    fuzz_strategies::calldata_strategy::fuzz_state_calldata, Evm, ASSUME_MAGIC_RETURN_CODE,
};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes},
};
use std::{
    cell::{RefCell, RefMut},
    io::Write,
    marker::PhantomData,
    rc::Rc,
};

pub use proptest::test_runner::{Config as FuzzConfig, Reason};
use proptest::{
    prelude::*,
    test_runner::{TestError, TestRunner},
};
use serde::{Deserialize, Serialize};

/// Wrapper around any [`Evm`](crate::Evm) implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided `TestRunner` contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
#[derive(Debug)]
pub struct FuzzedExecutor<'a, E, S> {
    evm: RefCell<&'a mut E>,
    runner: TestRunner,
    state: PhantomData<S>,
    sender: Address,
    pub state_weight: u32,
    pub random_weight: u32,
}

impl<'a, S, E: Evm<S>> FuzzedExecutor<'a, E, S> {
    pub fn into_inner(self) -> &'a mut E {
        self.evm.into_inner()
    }

    /// Returns a mutable reference to the fuzzer's internal EVM instance
    pub fn as_mut(&self) -> RefMut<'_, &'a mut E> {
        self.evm.borrow_mut()
    }

    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(
        evm: &'a mut E,
        runner: TestRunner,
        sender: Address,
        state_weight: u32,
        random_weight: u32,
    ) -> Self {
        Self {
            evm: RefCell::new(evm),
            runner,
            state: PhantomData,
            sender,
            state_weight,
            random_weight,
        }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case
    pub fn fuzz(
        &self,
        func: &Function,
        address: Address,
        should_fail: bool,
        abi: Option<&Abi>,
    ) -> FuzzTestResult<E::ReturnReason>
    where
        // We need to be able to clone the state so as to snapshot it and reset
        // it back after every test run, to have isolation of state across each
        // fuzz test run.
        S: Clone,
    {
        let mut strats = Vec::new();
        if self.random_weight > 0 {
            strats.push((self.random_weight, fuzz_state_calldata(func.clone(), None).boxed()))
        }

        // Snapshot the state before the test starts running
        let pre_test_state = self.evm.borrow().state().clone();
        let flattened_state = Rc::new(RefCell::new(self.evm.borrow().flatten_state()));

        // we dont shrink for state strategy
        if self.state_weight > 0 {
            strats.push((
                self.state_weight,
                fuzz_state_calldata(func.clone(), Some(flattened_state.clone()))
                    .no_shrink()
                    .boxed(),
            ))
        }

        if strats.is_empty() {
            panic!("Fuzz strategy weights were all 0. Please set at least one strategy weight to be above 0");
        }

        // stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzCase>> = RefCell::new(Default::default());

        let combined_strat = proptest::strategy::Union::new_weighted(strats);

        // stores the latest reason of a test call, this will hold the return reason of failed test
        // case if the runner failed
        let return_reason: RefCell<Option<E::ReturnReason>> = RefCell::new(None);
        let revert_reason = RefCell::new(None);

        let mut runner = self.runner.clone();
        tracing::debug!(func = ?func.name, should_fail, "fuzzing");
        let ret_calldata = RefCell::new(None);

        let test_error = runner
            .run(&combined_strat, |calldata| {
                let mut evm = self.evm.borrow_mut();
                // Before each test, we must reset to the initial state
                evm.reset(pre_test_state.clone());

                let (returndata, reason, gas, _) = evm
                    .call_raw(self.sender, address, calldata.clone(), 0.into(), false)
                    .expect("could not make raw evm call");

                // When assume cheat code is triggered return a special string "FOUNDRY::ASSUME"
                if returndata.as_ref() == ASSUME_MAGIC_RETURN_CODE {
                    let _ = return_reason.borrow_mut().insert(reason);
                    let err = "ASSUME: Too many rejects";
                    let _ = revert_reason.borrow_mut().insert(err.to_string());
                    return Err(TestCaseError::Reject(err.into()))
                }

                // We must check success before resetting the state, otherwise resetting the state
                // will also reset the `failed` state variable back to false.
                let success = evm.check_success(address, &reason, should_fail);

                // store the result of this test case
                let _ = return_reason.borrow_mut().insert(reason);

                if !success {
                    let revert =
                        foundry_utils::decode_revert(returndata.as_ref(), abi).unwrap_or_default();
                    let _ = revert_reason.borrow_mut().insert(revert);
                    
                    // because of how we do state selector, (totally random)
                    // we have to manually set the test_error data. Otherwise
                    // the way proptest works, makes it so the failing calldata wouldnt be the same
                    // as the test_error calldata. so we do this instead
                    let mut cd = ret_calldata.borrow_mut();
                    *cd = Some(calldata.clone());
                }

                // This will panic and get caught by the executor
                proptest::prop_assert!(
                    success,
                    "{}, expected failure: {}, reason: '{}'",
                    func.name,
                    should_fail,
                    match foundry_utils::decode_revert(returndata.as_ref(), abi) {
                        Ok(e) => e,
                        Err(e) => e.to_string(),
                    }
                );

                {
                    let mut t = flattened_state.borrow_mut();
                    (*t).extend(evm.flatten_state());
                    
                    returndata.as_ref().chunks(32).for_each(|chunk| {
                        let mut to_fill: [u8; 32] = [0; 32];
                        let _ = (&mut to_fill[..])
                            .write(chunk)
                            .expect("Chunk cannot be greater than 32 bytes");
                        (*t).insert(to_fill);
                    });
                }

                // push test case to the case set
                fuzz_cases.borrow_mut().push(FuzzCase { calldata, gas });

                Ok(())
            })
            .err()
            .map(|test_error| FuzzError {
                // selector strategy isnt reproducible, so we hack around that by using a refcell
                test_error:  match test_error {
                    TestError::Abort(msg) => TestError::Abort(msg),
                    TestError::Fail(msg, _cd) => {
                        TestError::Fail(msg, ret_calldata.into_inner().expect("Calldata must be set"))
                    }
                },
                return_reason: return_reason.into_inner().expect("Reason must be set"),
                revert_reason: revert_reason.into_inner().expect("Revert error string must be set"),
            });

        self.evm.borrow_mut().reset(pre_test_state);
        FuzzTestResult { cases: FuzzedCases::new(fuzz_cases.into_inner()), test_error }
    }
}

/// The outcome of a fuzz test
pub struct FuzzTestResult<Reason> {
    /// Every successful fuzz test case
    pub cases: FuzzedCases,
    /// if there was a case that resulted in an error, this contains the error and the return
    /// reason of the failed call
    pub test_error: Option<FuzzError<Reason>>,
}

impl<Reason> FuzzTestResult<Reason> {
    /// Returns `true` if all test cases succeeded
    pub fn is_ok(&self) -> bool {
        self.test_error.is_none()
    }

    /// Returns `true` if a test case failed
    pub fn is_err(&self) -> bool {
        self.test_error.is_some()
    }
}

pub struct FuzzError<Reason> {
    /// The proptest error occurred as a result of a test case
    pub test_error: TestError<Bytes>,
    /// The return reason of the offending call
    pub return_reason: Reason,
    /// The revert string of the offending call
    pub revert_reason: String,
}

/// Container type for all successful test cases
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FuzzedCases {
    cases: Vec<FuzzCase>,
}

impl FuzzedCases {
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
pub struct FuzzCase {
    /// The calldata used for this fuzz test
    pub calldata: Bytes,
    // Consumed gas
    pub gas: u64,
}

#[cfg(test)]
#[cfg(feature = "sputnik")]
mod tests {
    use super::*;

    use crate::{
        sputnik::helpers::{fuzzvm, vm},
        test_helpers::COMPILED,
        Evm,
    };

    #[test]
    fn prints_fuzzed_revert_reasons() {
        let mut evm = vm();

        let compiled = COMPILED.find("FuzzTests").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        let evm = fuzzvm(&mut evm);

        let func = compiled.abi.unwrap().function("testFuzzedRevert").unwrap();
        let res = evm.fuzz(func, addr, false, compiled.abi);
        let error = res.test_error.unwrap();
        let revert_reason = error.revert_reason;
        assert_eq!(revert_reason, "fuzztest-revert");
    }

    #[test]
    fn finds_fuzzed_state_revert() {
        let mut evm = vm();

        let compiled = COMPILED.find("FuzzTests").expect("could not find contract");
        let (addr, _, _, _) =
            evm.deploy(Address::zero(), compiled.bytecode().unwrap().clone(), 0.into()).unwrap();

        let evm = fuzzvm(&mut evm);

        let func = compiled.abi.unwrap().function("testFuzzedStateRevert").unwrap();
        let res = evm.fuzz(func, addr, false, compiled.abi);
        let error = res.test_error.unwrap();
        let revert_reason = error.revert_reason;
        assert_eq!(revert_reason, "fuzzstate-revert");
    }
}
