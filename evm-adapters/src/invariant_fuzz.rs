//! Fuzzing support abstracted over the [`Evm`](crate::Evm) used
use std::collections::BTreeMap;
use crate::Evm;
use crate::fuzz::*;

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


use crate::fuzz::strategies;

/// Wrapper around any [`Evm`](crate::Evm) implementor which provides fuzzing support using [`proptest`](https://docs.rs/proptest/1.0.0/proptest/).
///
/// After instantiation, calling `fuzz` will proceed to hammer the deployed smart contract with
/// inputs, until it finds a counterexample. The provided `TestRunner` contains all the
/// configuration which can be overridden via [environment variables](https://docs.rs/proptest/1.0.0/proptest/test_runner/struct.Config.html)
#[derive(Debug)]
pub struct InvariantExecutor<'a, E, S> {
    evm: RefCell<&'a mut E>,
    runner: TestRunner,
    state: PhantomData<S>,
    sender: Address,
    contracts: &'a BTreeMap<Address, (String, Abi)>,
}

impl<'a, S, E: Evm<S>> InvariantExecutor<'a, E, S> {
    pub fn into_inner(self) -> &'a mut E {
        self.evm.into_inner()
    }

    /// Returns a mutable reference to the fuzzer's internal EVM instance
    pub fn as_mut(&self) -> RefMut<'_, &'a mut E> {
        self.evm.borrow_mut()
    }

    /// Instantiates a fuzzed executor EVM given a testrunner
    pub fn new(evm: &'a mut E, runner: TestRunner, sender: Address, contracts: &'a BTreeMap<Address, (String, Abi)>,) -> Self {
        Self { evm: RefCell::new(evm), runner, state: PhantomData, sender, contracts }
    }

    /// Fuzzes the provided function, assuming it is available at the contract at `address`
    /// If `should_fail` is set to `true`, then it will stop only when there's a success
    /// test case.
    ///
    /// Returns a list of all the consumed gas and calldata of every fuzz case
    pub fn invariant_fuzz(
        &self,
        invariant_address: Address,
        abi: Option<&Abi>,
    ) -> Option<InvariantFuzzTestResult<E::ReturnReason>>
    where
        // We need to be able to clone the state so as to snapshot it and reset
        // it back after every test run, to have isolation of state across each
        // fuzz test run.
        S: Clone,
    {

        let invariants: Vec<Function>;
        if let Some(abi) = abi {
            invariants = abi.functions().filter(|func| func.name.starts_with("invariant")).cloned().collect()
        } else {
            return None;
        };

        let contracts: BTreeMap<Address, _> = self.contracts.clone().into_iter().filter(| (addr, _)| *addr != Address::from_slice(&hex::decode("7109709ECfa91a80626fF3989D68f67F5b1DD12D").unwrap()) && *addr !=  Address::from_slice(&hex::decode("000000000000000000636F6e736F6c652e6c6f67").unwrap())).collect();
        let strat = invariant_strat(15, contracts);

        // Snapshot the state before the test starts running
        let pre_test_state = self.evm.borrow().state().clone();

        // stores the consumed gas and calldata of every successful fuzz call
        let fuzz_cases: RefCell<Vec<FuzzCase>> = RefCell::new(Default::default());

        // stores the latest reason of a test call, this will hold the return reason of failed test
        // case if the runner failed
        let return_reason: RefCell<Option<E::ReturnReason>> = RefCell::new(None);
        let revert_reason = RefCell::new(None);
        let mut all_invars = BTreeMap::new();
        invariants.iter().for_each(|f| {
            all_invars.insert(f.name.to_string(), None);
        });
        let invariant_doesnt_hold = RefCell::new(all_invars);

        let mut runner = self.runner.clone();
        let _test_error = runner
            .run(&strat, |inputs| {
                let mut evm = self.evm.borrow_mut();
                // Before each test, we must reset to the initial state
                evm.reset(pre_test_state.clone());

                // println!("inputs len: {:?}", inputs.len());
                'all: for (address, calldata) in inputs.iter() {
                    // println!("address {:?} {:?}", address, hex::encode(&calldata));
                    let (_, reason, gas, _) = evm
                        .call_raw(self.sender, *address, calldata.clone(), 0.into(), false)
                        .expect("could not make raw evm call");

                    if !is_fail(*evm, &reason) {
                        // iterate over invariants, making sure they dont fail
                        for func in invariants.iter() {
                            let (retdata, status, _gas, _logs) = evm.call_unchecked(self.sender, invariant_address, &func, (), 0.into()).expect("EVM error");
                            if is_fail(*evm, &status) {
                                invariant_doesnt_hold.borrow_mut().insert(func.name.clone(), Some( InvariantFuzzError {
                                    test_error: proptest::test_runner::TestError::Fail(
                                        format!(
                                            "{}, reason: '{}'",
                                            func.name,
                                            match foundry_utils::decode_revert(retdata.as_ref(), abi) {
                                                Ok(e) => e,
                                                Err(e) => e.to_string(),
                                            }
                                        ).into(),
                                        inputs.clone()
                                    ),
                                    return_reason: status,
                                    revert_reason: foundry_utils::decode_revert(retdata.as_ref(), abi).unwrap_or_default(),
                                    addr: invariant_address,
                                    func: func.short_signature().into(),
                                    })
                                );
                                break 'all;
                            } else {
                                // This will panic and get caught by the executor
                                if !evm.check_success(invariant_address, &reason, false) {
                                    invariant_doesnt_hold.borrow_mut().insert(func.name.clone(), Some( InvariantFuzzError {
                                        test_error: proptest::test_runner::TestError::Fail(
                                            format!(
                                                "{}, reason: '{}'",
                                                func.name,
                                                match foundry_utils::decode_revert(retdata.as_ref(), abi) {
                                                    Ok(e) => e,
                                                    Err(e) => e.to_string(),
                                                }
                                            ).into(),
                                            inputs.clone()
                                        ),
                                        return_reason: status,
                                        revert_reason: foundry_utils::decode_revert(retdata.as_ref(), abi).unwrap_or_default(),
                                        addr: invariant_address,
                                        func: func.short_signature().into(),
                                    }));
                                    break 'all;
                                }
                            }
                        }
                        // push test case to the case set
                        fuzz_cases.borrow_mut().push(FuzzCase { calldata: calldata.clone(), gas });
                    } else {
                        // call failed, continue on   
                    }
                }

                Ok(())
            })
            .err()
            .map(|test_error| InvariantFuzzError {
                test_error,
                return_reason: return_reason.into_inner().expect("Reason must be set"),
                revert_reason: revert_reason.into_inner().expect("Revert error string must be set"),
                addr: invariant_address,
                func: ethers::prelude::Bytes::default(),
            });

        self.evm.borrow_mut().reset(pre_test_state.clone());

        Some(InvariantFuzzTestResult { invariants: invariant_doesnt_hold.into_inner(), cases: FuzzedCases::new(fuzz_cases.into_inner()) })
    }
}

/// The outcome of a fuzz test
pub struct InvariantFuzzTestResult<Reason> {
    pub invariants: BTreeMap<String, Option<InvariantFuzzError<Reason>>>,
    /// Every successful fuzz test case
    pub cases: FuzzedCases,
}

impl<Reason> InvariantFuzzTestResult<Reason> {
    /// Returns `true` if all test cases succeeded
    pub fn is_ok(&self) -> bool {
        !self.invariants.iter().any(|(_k, i)| i.is_some())
        // self.test_error.is_none()
    }

    /// Returns `true` if a test case failed
    pub fn is_err(&self) -> bool {
        self.invariants.iter().any(|(_k, i)| i.is_some())
    }
}

pub struct InvariantFuzzError<Reason> {
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

fn is_fail<S: Clone, E: Evm<S> + crate::Evm<S, ReturnReason = T>, T>(
    _evm: &mut E,
    status: &T,
) -> bool {
    <E as crate::Evm<S>>::is_fail(status)
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

pub fn invariant_strat(depth: usize, contracts: BTreeMap<Address, (String, Abi)>) -> BoxedStrategy<Vec<(Address, Bytes)>> {
    let iters = 1..depth+1;
    proptest::collection::vec(gen_call(contracts), iters).boxed()
}

fn gen_call(contracts: BTreeMap<Address, (String, Abi)>) -> BoxedStrategy<(Address, Bytes)> {
    let random_contract = select_random_contract(contracts);
    random_contract.prop_flat_map(move |(contract, abi)| {
        let func = select_random_function(abi);
        func.prop_flat_map(move |func| {
            fuzz_calldata(contract, func.clone())
        })
    }).boxed()
}

fn select_random_contract(contracts: BTreeMap<Address, (String, Abi)>) -> impl Strategy<Value = (Address, Abi)> {
    let selectors = any::<prop::sample::Selector>();
    selectors.prop_map(move |selector| {
        let res = selector.select(&contracts);
        (*res.0, res.1.1.clone())
    })
}

fn select_random_function(abi: Abi) -> impl Strategy<Value = Function> {
    let selectors = any::<prop::sample::Selector>();
    let possible_funcs: Vec<ethers::abi::Function> = abi.functions().filter(|func| !matches!(func.state_mutability, ethers::abi::StateMutability::Pure | ethers::abi::StateMutability::View)).cloned().collect();
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
        ParamType::Uint(n) => {
            strategies::UintStrategy::new(*n, vec![]).prop_map(|x| x.into_token()).boxed()
        }
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
