#![allow(unknown_lints, clippy::incompatible_msrv, missing_docs)]

use alloy_dyn_abi::{DynSolType, Specifier};
use alloy_sol_type_parser::TypeSpecifier;
use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use rand::seq::SliceRandom;
use std::{hint::black_box, time::Duration};

const KEYWORDS: &[&str] =
    &["address", "bool", "string", "bytes", "bytes32", "uint", "uint256", "int", "int256"];
const COMPLEX: &[&str] = &[
    "((uint104,bytes,bytes8,bytes7,address,bool,address,int256,int32,bytes1,uint56,int136),uint80,uint104,address,bool,bytes14,int16,address,string,uint176,uint72,(uint120,uint192,uint256,int232,bool,bool,bool,bytes5,int56,address,uint224,int248,bytes10,int48,int8),string,string,bool,bool)",
    "(address,string,(bytes,int48,bytes30,bool,address,bytes30,int48,address,bytes17,bool,uint32),bool,address,bytes28,bytes25,uint136)",
    "(uint168,bytes21,address,(bytes,bool,string,address,bool,string,bytes,uint232,int128,int64,uint96,bytes7,int136),bool,uint200[5],bool,bytes,uint240,address,address,bytes15,bytes)"
];

fn parse(c: &mut Criterion) {
    let mut g = group(c, "parse");
    let rng = &mut rand::thread_rng();

    g.bench_function("keywords", |b| {
        b.iter(|| {
            let kw = *KEYWORDS.choose(rng).unwrap();
            TypeSpecifier::parse(black_box(kw)).unwrap()
        });
    });
    g.bench_function("complex", |b| {
        b.iter(|| {
            let complex = *COMPLEX.choose(rng).unwrap();
            TypeSpecifier::parse(black_box(complex)).unwrap()
        });
    });

    g.finish();
}

fn resolve(c: &mut Criterion) {
    let mut g = group(c, "resolve");
    let rng = &mut rand::thread_rng();

    g.bench_function("keywords", |b| {
        let parsed_keywords =
            KEYWORDS.iter().map(|s| TypeSpecifier::parse(s).unwrap()).collect::<Vec<_>>();
        let parsed_keywords = parsed_keywords.as_slice();
        b.iter(|| {
            let kw = parsed_keywords.choose(rng).unwrap();
            black_box(kw).resolve().unwrap()
        });
    });
    g.bench_function("complex", |b| {
        let complex = COMPLEX.iter().map(|s| TypeSpecifier::parse(s).unwrap()).collect::<Vec<_>>();
        let complex = complex.as_slice();
        b.iter(|| {
            let complex = complex.choose(rng).unwrap();
            black_box(complex).resolve().unwrap()
        });
    });

    g.finish();
}

fn format(c: &mut Criterion) {
    let mut g = group(c, "format");
    let rng = &mut rand::thread_rng();

    g.bench_function("keywords", |b| {
        let keyword_types =
            KEYWORDS.iter().map(|s| DynSolType::parse(s).unwrap()).collect::<Vec<_>>();
        let keyword_types = keyword_types.as_slice();
        b.iter(|| {
            let kw = unsafe { keyword_types.choose(rng).unwrap_unchecked() };
            black_box(kw).sol_type_name()
        });
    });
    g.bench_function("complex", |b| {
        let complex_types =
            COMPLEX.iter().map(|s| DynSolType::parse(s).unwrap()).collect::<Vec<_>>();
        let complex_types = complex_types.as_slice();
        b.iter(|| {
            let complex = unsafe { complex_types.choose(rng).unwrap_unchecked() };
            black_box(complex).sol_type_name()
        });
    });

    g.finish();
}

fn group<'a>(c: &'a mut Criterion, group_name: &str) -> BenchmarkGroup<'a, WallTime> {
    let mut g = c.benchmark_group(group_name);
    g.noise_threshold(0.03)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(200);
    g
}

criterion_group!(benches, parse, resolve, format);
criterion_main!(benches);
