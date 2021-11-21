use crate::Evm;
use ethers::{
    abi::{Function, ParamType, Token, Tokenizable},
    types::{Address, Bytes, Sign, I256, U256},
};
use std::{
    cell::{RefCell, RefMut},
    marker::PhantomData,
};

use proptest::{
    prelude::*,
    test_runner::{TestError, TestRunner},
};

pub use proptest::test_runner::Config as FuzzConfig;

#[derive(Debug)]
pub struct FuzzedExecutor<'a, E, S> {
    evm: RefCell<&'a mut E>,
    runner: TestRunner,
    state: PhantomData<S>,
    sender: Address,
}

impl<'a, S, E: Evm<S>> FuzzedExecutor<'a, E, S> {
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
    pub fn fuzz(
        &self,
        func: &Function,
        address: Address,
        should_fail: bool,
    ) -> Result<(), TestError<Bytes>>
    where
        // We need to be able to clone the state so as to snapshot it and reset
        // it back after every test run, to have isolation of state across each
        // fuzz test run.
        S: Clone,
    {
        let strat = fuzz_calldata(func);

        // Snapshot the state before the test starts running
        let pre_test_state = self.evm.borrow().state().clone();

        let mut runner = self.runner.clone();
        tracing::debug!(func = ?func.name, should_fail, "fuzzing");
        runner.run(&strat, |calldata| {
            let mut evm = self.evm.borrow_mut();

            // Before each test, we must reset to the initial state
            evm.reset(pre_test_state.clone());

            let (returndata, reason, _, _) = evm
                .call_raw(self.sender, address, calldata, 0.into(), false)
                .expect("could not make raw evm call");

            // We must check success before resetting the state, otherwise resetting the state
            // will also reset the `failed` state variable back to false.
            let success = evm.check_success(address, &reason, should_fail);

            // This will panic and get caught by the executor
            proptest::prop_assert!(
                success,
                "{}, expected failure: {}, reason: '{}'",
                func.name,
                should_fail,
                foundry_utils::decode_revert(returndata.as_ref())?
            );

            Ok(())
        })
    }
}

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

fn fuzz_param(param: &ParamType) -> impl Strategy<Value = Token> {
    match param {
        ParamType::Address => {
            // The key to making this work is the `boxed()` call which type erases everything
            // https://altsysrq.github.io/proptest-book/proptest/tutorial/transforming-strategies.html
            any::<[u8; 20]>().prop_map(|x| Address::from_slice(&x).into_token()).boxed()
        }
        ParamType::Bytes => any::<Vec<u8>>().prop_map(|x| Bytes::from(x).into_token()).boxed(),
        ParamType::Int(n) => match n / 8 {
            1 => any::<i8>().prop_map(|x| x.into_token()).boxed(),
            2 => any::<i16>().prop_map(|x| x.into_token()).boxed(),
            3..=4 => any::<i32>().prop_map(|x| x.into_token()).boxed(),
            5..=8 => any::<i64>().prop_map(|x| x.into_token()).boxed(),
            9..=16 => any::<i128>().prop_map(|x| x.into_token()).boxed(),
            17..=32 => (any::<bool>(), any::<[u8; 32]>())
                .prop_filter_map("i256s cannot overflow", |(sign, bytes)| {
                    let sign = if sign { Sign::Positive } else { Sign::Negative };
                    I256::checked_from_sign_and_abs(sign, U256::from(bytes)).map(|x| x.into_token())
                })
                .boxed(),
            _ => panic!("unsupported solidity type int{}", n),
        },
        ParamType::Uint(n) => match n / 8 {
            1 => any::<u8>().prop_map(|x| x.into_token()).boxed(),
            2 => any::<u16>().prop_map(|x| x.into_token()).boxed(),
            3..=4 => any::<u32>().prop_map(|x| x.into_token()).boxed(),
            5..=8 => any::<u64>().prop_map(|x| x.into_token()).boxed(),
            9..=16 => any::<u128>().prop_map(|x| x.into_token()).boxed(),
            17..=32 => any::<[u8; 32]>().prop_map(|x| U256::from(&x).into_token()).boxed(),
            _ => panic!("unsupported solidity type uint{}", n),
        },
        ParamType::Bool => any::<bool>().prop_map(|x| x.into_token()).boxed(),
        ParamType::String => any::<String>().prop_map(|x| x.into_token()).boxed(),
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
