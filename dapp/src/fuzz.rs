use ethers::{
    abi::{Function, ParamType, Token, Tokenizable},
    types::{Address, Bytes, Sign, I256, U256},
};

use proptest::prelude::*;

pub fn fuzz_calldata(func: &Function) -> impl Strategy<Value = Bytes> + '_ {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    let strats = func.inputs.iter().map(|input| fuzz_param(&input.kind)).collect::<Vec<_>>();

    strats.prop_map(move |tokens| func.encode_input(&tokens).unwrap().into())
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
