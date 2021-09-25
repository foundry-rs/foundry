use ethers::{
    abi::{Function, ParamType, Token, Tokenizable},
    types::{Address, Bytes, U256},
};

use proptest::prelude::*;

pub fn fuzz_calldata(func: &Function) -> impl Strategy<Value = Bytes> + '_ {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    let strats = func.inputs.iter().map(|input| fuzz_param(&input.kind)).collect::<Vec<_>>();

    strats.prop_map(move |tokens| func.encode_input(&tokens).unwrap().into())
}

fn fuzz_param(param: &ParamType) -> impl Strategy<Value = Token> {
    match param {
        ParamType::Address => {
            // The key to making this work is the `boxed()` call which type erases everything
            // https://altsysrq.github.io/proptest-book/proptest/tutorial/transforming-strategies.html
            any::<[u8; 20]>().prop_map(|x| Address::from_slice(&x).into_token()).boxed()
        }
        ParamType::Uint(n) => match n / 8 {
            1 => any::<u8>().prop_map(|x| x.into_token()).boxed(),
            2 => any::<u16>().prop_map(|x| x.into_token()).boxed(),
            3..=4 => any::<u32>().prop_map(|x| x.into_token()).boxed(),
            5..=8 => any::<u64>().prop_map(|x| x.into_token()).boxed(),
            9..=16 => any::<u128>().prop_map(|x| x.into_token()).boxed(),
            17..=32 => any::<[u8; 32]>().prop_map(|x| U256::from(&x).into_token()).boxed(),
            _ => panic!("unsupported solidity type uint{}", n),
        },
        ParamType::String => any::<String>().prop_map(|x| x.into_token()).boxed(),
        ParamType::Bytes => any::<Vec<u8>>().prop_map(|x| Bytes::from(x).into_token()).boxed(),
        // TODO: Implement the rest of the strategies
        _ => unimplemented!(),
    }
}
