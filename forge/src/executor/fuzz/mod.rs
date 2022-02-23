mod strategies;

// TODO Port when we have cheatcodes again
//use crate::{Evm, ASSUME_MAGIC_RETURN_CODE};
use crate::executor::{Executor, RawCallResult};
use ethers::{
    abi::{Abi, Function},
    types::{Address, Bytes},
};
use revm::{db::DatabaseRef, Return};
use strategies::fuzz_calldata;

pub use proptest::test_runner::{Config as FuzzConfig, Reason};
use proptest::test_runner::{TestError, TestRunner};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// Wrapper around an [`Executor`] which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided [`TestRunner`] contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
pub struct FuzzedExecutor<'a, DB: DatabaseRef> {
    /// The VM
    executor: &'a Executor<DB>,
    /// The fuzzer
    runner: TestRunner,
    /// The account that calls tests
    sender: Address,
}

impl<'a, DB> FuzzedExecutor<'a, DB>
where
    DB: DatabaseRef,
{
    /// Instantiates a fuzzed executor given a testrunner
    pub fn new(executor: &'a Executor<DB>, runner: TestRunner, sender: Address) -> Self {
        Self { executor, runner, sender }
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
    ) -> FuzzTestResult {
        let strat = fuzz_calldata(func);

        // Stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzCase>> = RefCell::new(Default::default());

        // Stores the latest return and revert reason of a test call
        let return_reason: RefCell<Option<Return>> = RefCell::new(None);
        let revert_reason = RefCell::new(None);

        let mut runner = self.runner.clone();
        tracing::debug!(func = ?func.name, should_fail, "fuzzing");
        let test_error = runner
            .run(&strat, |calldata| {
                let RawCallResult { status, result, gas, state_changeset, .. } = self
                    .executor
                    .call_raw(self.sender, address, calldata.0.clone(), 0.into())
                    .expect("could not make raw evm call");

                // When assume cheat code is triggered return a special string "FOUNDRY::ASSUME"
                // TODO: Re-implement when cheatcodes are ported
                /*if returndata.as_ref() == ASSUME_MAGIC_RETURN_CODE {
                    let _ = return_reason.borrow_mut().insert(reason);
                    let err = "ASSUME: Too many rejects";
                    let _ = revert_reason.borrow_mut().insert(err.to_string());
                    return Err(TestCaseError::Reject(err.into()));
                }*/

                let success = self.executor.is_success(
                    address,
                    status,
                    state_changeset.expect("we should have a state changeset"),
                    should_fail,
                );

                // Store the result of this test case
                let _ = return_reason.borrow_mut().insert(status);
                if !success {
                    let revert =
                        foundry_utils::decode_revert(result.as_ref(), abi).unwrap_or_default();
                    let _ = revert_reason.borrow_mut().insert(revert);
                }

                // This will panic and get caught by the executor
                proptest::prop_assert!(
                    success,
                    "{}, expected failure: {}, reason: '{}'",
                    func.name,
                    should_fail,
                    match foundry_utils::decode_revert(result.as_ref(), abi) {
                        Ok(e) => e,
                        Err(e) => e.to_string(),
                    }
                );

                // Push test case to the case set
                fuzz_cases.borrow_mut().push(FuzzCase { calldata, gas });
                Ok(())
            })
            .err()
            .map(|test_error| FuzzError {
                test_error,
                return_reason: return_reason.into_inner().expect("Reason must be set"),
                revert_reason: revert_reason.into_inner().expect("Revert error string must be set"),
            });

        FuzzTestResult { cases: FuzzedCases::new(fuzz_cases.into_inner()), test_error }
    }
}

/// The outcome of a fuzz test
pub struct FuzzTestResult {
    /// Every successful fuzz test case
    pub cases: FuzzedCases,
    /// if there was a case that resulted in an error, this contains the error and the return
    /// reason of the failed call
    pub test_error: Option<FuzzError>,
}

impl FuzzTestResult {
    /// Returns `true` if all test cases succeeded
    pub fn is_ok(&self) -> bool {
        self.test_error.is_none()
    }

    /// Returns `true` if a test case failed
    pub fn is_err(&self) -> bool {
        self.test_error.is_some()
    }
}

pub struct FuzzError {
    /// The proptest error occurred as a result of a test case
    pub test_error: TestError<Bytes>,
    /// The return reason of the offending call
    pub return_reason: Return,
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

    /// Returns the case with the highest gas usage
    pub fn highest(&self) -> Option<&FuzzCase> {
        self.cases.last()
    }

    /// Returns the case with the lowest gas usage
    pub fn lowest(&self) -> Option<&FuzzCase> {
        self.cases.first()
    }

    /// Returns the highest amount of gas spent on a fuzz case
    pub fn highest_gas(&self) -> u64 {
        self.highest().map(|c| c.gas).unwrap_or_default()
    }

    /// Returns the lowest amount of gas spent on a fuzz case
    pub fn lowest_gas(&self) -> u64 {
        self.lowest().map(|c| c.gas).unwrap_or_default()
    }
}

/// Data of a single fuzz test case
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FuzzCase {
    /// The calldata used for this fuzz test
    pub calldata: Bytes,
    /// Consumed gas
    pub gas: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_helpers::{fuzz_executor, test_executor, COMPILED};

    #[test]
    fn prints_fuzzed_revert_reasons() {
        let mut executor = test_executor();

        let compiled = COMPILED.find("FuzzTests").expect("could not find contract");
        let (addr, _, _, _) = executor
            .deploy(Address::zero(), compiled.bytecode().unwrap().0.clone(), 0.into())
            .unwrap();

        let executor = fuzz_executor(&executor);

        let func = compiled.abi.unwrap().function("testFuzzedRevert").unwrap();
        let res = executor.fuzz(func, addr, false, compiled.abi);
        let error = res.test_error.unwrap();
        let revert_reason = error.revert_reason;
        assert_eq!(revert_reason, "fuzztest-revert");
    }
}
