//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use crate::Evm;
use ethers::{
    abi::{Abi, Function, ParamType, Token, Tokenizable},
    types::{Address, Bytes, I256, U256},
};
use std::{
    cell::{RefCell, RefMut},
    marker::PhantomData,
};

pub use proptest::test_runner::Config as FuzzConfig;
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
    pub fn new(evm: &'a mut E, runner: TestRunner, sender: Address) -> Self {
        Self { evm: RefCell::new(evm), runner, state: PhantomData, sender }
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
        let strat = fuzz_calldata(func);

        // Snapshot the state before the test starts running
        let pre_test_state = self.evm.borrow().state().clone();

        // stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzCase>> = RefCell::new(Default::default());

        // stores the latest reason of a test call, this will hold the return reason of failed test
        // case if the runner failed
        let return_reason: RefCell<Option<E::ReturnReason>> = RefCell::new(None);
        let revert_reason = RefCell::new(None);

        let mut runner = self.runner.clone();
        tracing::debug!(func = ?func.name, should_fail, "fuzzing");
        let test_error = runner
            .run(&strat, |calldata| {
                let mut evm = self.evm.borrow_mut();
                // Before each test, we must reset to the initial state
                evm.reset(pre_test_state.clone());

                let (returndata, reason, gas, _) = evm
                    .call_raw(self.sender, address, calldata.clone(), 0.into(), false)
                    .expect("could not make raw evm call");

                // We must check success before resetting the state, otherwise resetting the state
                // will also reset the `failed` state variable back to false.
                let success = evm.check_success(address, &reason, should_fail);

                // store the result of this test case
                let _ = return_reason.borrow_mut().insert(reason);

                if !success {
                    let revert =
                        foundry_utils::decode_revert(returndata.as_ref(), abi).unwrap_or_default();
                    let _ = revert_reason.borrow_mut().insert(revert);
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

                // push test case to the case set
                fuzz_cases.borrow_mut().push(FuzzCase { calldata, gas });

                Ok(())
            })
            .err()
            .map(|test_error| FuzzError {
                test_error,
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

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_calldata(func: &Function) -> impl Strategy<Value = Bytes> + '_ {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    let strats = func.inputs.iter().map(|input| fuzz_param(&input.kind)).collect::<Vec<_>>();

    strats.prop_map(move |tokens| {
        tracing::trace!(input = ?tokens);
        func.encode_input(&tokens).unwrap().into()
    })
}

/// The max length of arrays we fuzz for is 256.
const MAX_ARRAY_LEN: usize = 256;

/// Given an ethabi parameter type, returns a proptest strategy for generating values for that
/// datatype. Works with ABI Encoder v2 tuples.
fn fuzz_param(param: &ParamType) -> impl Strategy<Value = Token> {
    match param {
        ParamType::Address => {
            // The key to making this work is the `boxed()` call which type erases everything
            // https://altsysrq.github.io/proptest-book/proptest/tutorial/transforming-strategies.html
            any::<[u8; 20]>().prop_map(|x| Address::from_slice(&x).into_token()).boxed()
        }
        ParamType::Bytes => any::<Vec<u8>>().prop_map(|x| Bytes::from(x).into_token()).boxed(),
        // For ints and uints we sample from a U256, then wrap it to the correct size with a
        // modulo operation. Note that this introduces modulo bias, but it can be removed with
        // rejection sampling if it's determined the bias is too severe. Rejection sampling may
        // slow down tests as it resamples bad values, so may want to benchmark the performance
        // hit and weigh that against the current bias before implementing
        ParamType::Int(n) => match n / 8 {
            32 => any::<[u8; 32]>()
                .prop_map(move |x| I256::from_raw(U256::from(&x)).into_token())
                .boxed(),
            y @ 1..=31 => any::<[u8; 32]>()
                .prop_map(move |x| {
                    // Generate a uintN in the correct range, then shift it to the range of intN
                    // by subtracting 2^(N-1)
                    let uint = U256::from(&x) % U256::from(2).pow(U256::from(y * 8));
                    let max_int_plus1 = U256::from(2).pow(U256::from(y * 8 - 1));
                    let num = I256::from_raw(uint.overflowing_sub(max_int_plus1).0);
                    num.into_token()
                })
                .boxed(),
            _ => panic!("unsupported solidity type int{}", n),
        },
        ParamType::Uint(n) => match n / 8 {
            32 => any::<[u8; 32]>().prop_map(move |x| U256::from(&x).into_token()).boxed(),
            y @ 1..=31 => any::<[u8; 32]>()
                .prop_map(move |x| {
                    (U256::from(&x) % (U256::from(2).pow(U256::from(y * 8)))).into_token()
                })
                .boxed(),
            _ => panic!("unsupported solidity type uint{}", n),
        },
        ParamType::Bool => any::<bool>().prop_map(|x| x.into_token()).boxed(),
        ParamType::String => any::<Vec<u8>>()
            .prop_map(|x| Token::String(unsafe { std::str::from_utf8_unchecked(&x).to_string() }))
            .boxed(),
        ParamType::Array(param) => proptest::collection::vec(fuzz_param(param), 0..MAX_ARRAY_LEN)
            .prop_map(Token::Array)
            .boxed(),
        ParamType::FixedBytes(size) => (0..*size as u64)
            .map(|_| any::<u8>())
            .collect::<Vec<_>>()
            .prop_map(Token::FixedBytes)
            .boxed(),
        ParamType::FixedArray(param, size) => (0..*size as u64)
            .map(|_| fuzz_param(param).prop_map(|param| param.into_token()))
            .collect::<Vec<_>>()
            .prop_map(Token::FixedArray)
            .boxed(),
        ParamType::Tuple(params) => {
            params.iter().map(fuzz_param).collect::<Vec<_>>().prop_map(Token::Tuple).boxed()
        }
    }
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
}
