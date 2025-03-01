#![allow(unknown_lints, clippy::incompatible_msrv, missing_docs)]

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{hex, Uint, U256};
use alloy_sol_types::{sol, sol_data, SolType, SolValue};
use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use std::{hint::black_box, time::Duration};

fn ethabi_encode(c: &mut Criterion) {
    let mut g = group(c, "ethabi/encode");

    g.bench_function("single", |b| {
        let input = encode_single_input();
        b.iter(|| {
            let token = ethabi::Token::String(input.clone());
            ethabi::encode(&[black_box(token)])
        });
    });

    g.bench_function("struct", |b| {
        let tokens = encode_struct_input_tokens();
        b.iter(|| ethabi::encode(black_box(&tokens)));
    });

    g.finish();
}

fn ethabi_decode(c: &mut Criterion) {
    let mut g = group(c, "ethabi/decode");

    g.bench_function("word", |b| {
        let input = decode_word_input();
        b.iter(|| {
            let ty = ethabi::ParamType::Uint(256);
            ethabi::decode(&[ty], black_box(&input)).unwrap()
        });
    });

    g.bench_function("dynamic", |b| {
        let input = decode_dynamic_input();
        b.iter(|| {
            let ty = ethabi::ParamType::String;
            ethabi::decode(&[ty], black_box(&input)).unwrap()
        });
    });

    g.finish();
}

fn dyn_abi_encode(c: &mut Criterion) {
    let mut g = group(c, "dyn-abi/encode");

    g.bench_function("single", |b| {
        let input = encode_single_input();
        b.iter(|| {
            let value = DynSolValue::String(input.clone());
            black_box(value).abi_encode()
        });
    });

    g.bench_function("struct", |b| {
        let input = encode_struct_sol_values();
        let input = DynSolValue::Tuple(input.to_vec());
        b.iter(|| black_box(&input).abi_encode_sequence());
    });

    g.finish();
}

fn dyn_abi_decode(c: &mut Criterion) {
    let mut g = group(c, "dyn-abi/decode");

    g.bench_function("word", |b| {
        let ty = DynSolType::Uint(256);
        let input = decode_word_input();
        b.iter(|| ty.abi_decode(black_box(&input)).unwrap());
    });

    g.bench_function("dynamic", |b| {
        let ty = DynSolType::String;
        let input = decode_dynamic_input();
        b.iter(|| ty.abi_decode(black_box(&input)).unwrap());
    });

    g.finish();
}

fn sol_types_encode(c: &mut Criterion) {
    let mut g = group(c, "sol-types/encode");

    g.bench_function("single", |b| {
        let input = encode_single_input();
        b.iter(|| black_box(&input).abi_encode());
    });

    g.bench_function("struct", |b| {
        let input = encode_struct_input();
        b.iter(|| black_box(&input).abi_encode());
    });

    g.finish();
}

fn sol_types_decode(c: &mut Criterion) {
    let mut g = group(c, "sol-types/decode");

    g.bench_function("word", |b| {
        let input = decode_word_input();
        b.iter(|| sol_data::Uint::<256>::abi_decode(black_box(&input), false).unwrap());
    });

    g.bench_function("dynamic", |b| {
        let input = decode_dynamic_input();
        b.iter(|| sol_data::String::abi_decode(black_box(&input), false).unwrap());
    });

    g.finish();
}

sol! {
    /// UniswapV3's `SwapRouter::ExactInputSingleParams`:
    /// <https://github.com/Uniswap/v3-periphery/blob/6cce88e63e176af1ddb6cc56e029110289622317/contracts/interfaces/ISwapRouter.sol#L10C10-L19>
    struct Input {
        address tokenIn;
        address tokenOut;
        uint24 fee;
        address recipient;
        uint256 deadline;
        uint256 amountIn;
        uint256 amountOutMinimum;
        uint160 sqrtPriceLimitX96;
    }
}

fn encode_single_input() -> String {
    String::from("Hello World!")
}

fn encode_struct_input() -> Input {
    Input {
        tokenIn: hex!("C02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").into(),
        tokenOut: hex!("955d5c14C8D4944dA1Ea7836bd44D54a8eC35Ba1").into(),
        fee: Uint::from(10000),
        recipient: hex!("299A299A22F8C7397d9DB3702439069d951AeA74").into(),
        deadline: U256::from(1685523099_u64),
        amountIn: U256::from(10000000000000000000_u128),
        amountOutMinimum: U256::from(836797564735606450550734848_u128),
        sqrtPriceLimitX96: Uint::ZERO,
    }
}

fn encode_struct_input_tokens() -> [ethabi::Token; 8] {
    let input = encode_struct_input();
    [
        ethabi::Token::Address(input.tokenIn.0 .0.into()),
        ethabi::Token::Address(input.tokenOut.0 .0.into()),
        ethabi::Token::Uint(input.fee.to::<u64>().into()),
        ethabi::Token::Address(input.recipient.0 .0.into()),
        ethabi::Token::Uint(ethabi::Uint::from_big_endian(&input.deadline.to_be_bytes_vec())),
        ethabi::Token::Uint(ethabi::Uint::from_big_endian(&input.amountIn.to_be_bytes_vec())),
        ethabi::Token::Uint(ethabi::Uint::from_big_endian(
            &input.amountOutMinimum.to_be_bytes_vec(),
        )),
        ethabi::Token::Uint(ethabi::Uint::from_big_endian(
            &input.sqrtPriceLimitX96.to_be_bytes_vec(),
        )),
    ]
}

fn encode_struct_sol_values() -> [DynSolValue; 8] {
    let input = encode_struct_input();
    [
        input.tokenIn.into(),
        input.tokenOut.into(),
        input.fee.to::<u64>().into(),
        input.recipient.into(),
        input.deadline.into(),
        input.amountIn.into(),
        input.amountOutMinimum.into(),
        input.sqrtPriceLimitX96.to::<U256>().into(),
    ]
}

fn decode_word_input() -> Vec<u8> {
    vec![0u8; 32]
}

fn decode_dynamic_input() -> Vec<u8> {
    hex!(
        "0000000000000000000000000000000000000000000000000000000000000020"
        "000000000000000000000000000000000000000000000000000000000000000c"
        "48656c6c6f20576f726c64210000000000000000000000000000000000000000"
    )
    .to_vec()
}

fn group<'a>(c: &'a mut Criterion, group_name: &str) -> BenchmarkGroup<'a, WallTime> {
    let mut g = c.benchmark_group(group_name);
    g.noise_threshold(0.03)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(200);
    g
}

criterion_group!(
    benches,
    ethabi_encode,
    ethabi_decode,
    dyn_abi_encode,
    dyn_abi_decode,
    sol_types_encode,
    sol_types_decode,
);
criterion_main!(benches);
