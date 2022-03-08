use crate::fuzz_strategies::uint_strategy::*;
use ethers::{
    abi::{Function, ParamType, Token, Tokenizable},
    types::{Address, Bytes, I256, U256},
};
use proptest::prelude::{Strategy, *};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_state_calldata(
    func: Function,
    state: Option<Rc<RefCell<HashSet<[u8; 32]>>>>,
) -> impl Strategy<Value = Bytes> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    // let strategy = proptest::sample::select(state.clone().into_iter().collect::<Vec<[u8;
    // 32]>>());
    let strats = func
        .inputs
        .iter()
        .map(|input| fuzz_param_with_input(&input.kind, state.clone()))
        .collect::<Vec<_>>();

    strats.prop_map(move |tokens| {
        tracing::trace!(input = ?tokens);
        func.encode_input(&tokens).unwrap().into()
    })
}

/// The max length of arrays we fuzz for is 256.
const MAX_ARRAY_LEN: usize = 256;

fn fuzz_param_with_input(
    param: &ParamType,
    state: Option<Rc<RefCell<HashSet<[u8; 32]>>>>,
) -> impl Strategy<Value = Token> {
    // The key to making this work is the `boxed()` call which type erases everything
    // https://altsysrq.github.io/proptest-book/proptest/tutorial/transforming-strategies.html
    if let Some(state) = state {
        state_fuzz(param, state)
    } else {
        match param {
            ParamType::Address => {
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
                UintStrategy::new(*n, vec![]).prop_map(|x| x.into_token()).boxed()
            }
            ParamType::Bool => any::<bool>().prop_map(|x| x.into_token()).boxed(),
            ParamType::String => any::<Vec<u8>>()
                .prop_map(|x| {
                    Token::String(unsafe { std::str::from_utf8_unchecked(&x).to_string() })
                })
                .boxed(),
            ParamType::Array(param) => {
                proptest::collection::vec(fuzz_param_with_input(param, None), 0..MAX_ARRAY_LEN)
                    .prop_map(Token::Array)
                    .boxed()
            }
            ParamType::FixedBytes(size) => (0..*size as u64)
                .map(|_| any::<u8>())
                .collect::<Vec<_>>()
                .prop_map(Token::FixedBytes)
                .boxed(),
            ParamType::FixedArray(param, size) => (0..*size as u64)
                .map(|_| fuzz_param_with_input(param, None).prop_map(|param| param.into_token()))
                .collect::<Vec<_>>()
                .prop_map(Token::FixedArray)
                .boxed(),
            ParamType::Tuple(params) => params
                .iter()
                .map(|p| fuzz_param_with_input(p, None))
                .collect::<Vec<_>>()
                .prop_map(Token::Tuple)
                .boxed(),
        }
    }
}

fn state_fuzz(param: &ParamType, state: Rc<RefCell<HashSet<[u8; 32]>>>) -> BoxedStrategy<Token> {
    let selectors = any::<prop::sample::Selector>();
    match param {
        ParamType::Address => selectors
            .prop_map(move |selector| {
                let x = *selector.select(&*state.borrow());
                Address::from_slice(&x[12..]).into_token()
            })
            .boxed(),
        ParamType::Bytes => selectors
            .prop_map(move |selector| {
                let x = *selector.select(&*state.borrow());
                Bytes::from(x).into_token()
            })
            .boxed(),
        ParamType::Int(n) => match n / 8 {
            32 => selectors
                .prop_map(move |selector| {
                    let x = *selector.select(&*state.borrow());
                    I256::from_raw(U256::from(x)).into_token()
                })
                .boxed(),
            y @ 1..=31 => selectors
                .prop_map(move |selector| {
                    let x = *selector.select(&*state.borrow());
                    // Generate a uintN in the correct range, then shift it to the range of intN
                    // by subtracting 2^(N-1)
                    let uint = U256::from(x) % U256::from(2).pow(U256::from(y * 8));
                    let max_int_plus1 = U256::from(2).pow(U256::from(y * 8 - 1));
                    let num = I256::from_raw(uint.overflowing_sub(max_int_plus1).0);
                    num.into_token()
                })
                .boxed(),
            _ => panic!("unsupported solidity type int{}", n),
        },
        ParamType::Uint(n) => match n / 8 {
            32 => selectors
                .prop_map(move |selector| {
                    let x = *selector.select(&*state.borrow());
                    U256::from(x).into_token()
                })
                .boxed(),
            y @ 1..=31 => selectors
                .prop_map(move |selector| {
                    let x = *selector.select(&*state.borrow());
                    (U256::from(x) % (U256::from(2).pow(U256::from(y * 8)))).into_token()
                })
                .boxed(),
            _ => panic!("unsupported solidity type uint{}", n),
        },
        ParamType::Bool => selectors
            .prop_map(move |selector| {
                let x = *selector.select(&*state.borrow());
                Token::Bool(x[31] == 1)
            })
            .boxed(),
        ParamType::String => selectors
            .prop_map(move |selector| {
                let x = *selector.select(&*state.borrow());
                Token::String(unsafe { std::str::from_utf8_unchecked(&x).to_string() })
            })
            .boxed(),
        ParamType::Array(param) => {
            proptest::collection::vec(fuzz_param_with_input(param, Some(state)), 0..MAX_ARRAY_LEN)
                .prop_map(Token::Array)
                .boxed()
        }
        ParamType::FixedBytes(size) => {
            // we have to clone outside the prop_map to satisfy lifetime constraints
            let v = *size;
            selectors
                .prop_map(move |selector| {
                    let x = *selector.select(&*state.borrow());
                    let val = x[32 - v..].to_vec();
                    Token::FixedBytes(val)
                })
                .boxed()
        }
        ParamType::FixedArray(param, size) => (0..*size as u64)
            .map(|_| {
                fuzz_param_with_input(param, Some(state.clone()))
                    .prop_map(|param| param.into_token())
            })
            .collect::<Vec<_>>()
            .prop_map(Token::FixedArray)
            .boxed(),
        ParamType::Tuple(params) => params
            .iter()
            .map(|p| fuzz_param_with_input(p, Some(state.clone())))
            .collect::<Vec<_>>()
            .prop_map(Token::Tuple)
            .boxed(),
    }
}
