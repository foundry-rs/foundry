#![allow(unknown_lints, clippy::incompatible_msrv)]

use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, Criterion,
};
use std::{hint::black_box, time::Duration};

fn serde(c: &mut Criterion) {
    let mut g = c.benchmark_group("serde");
    g.warm_up_time(Duration::from_secs(1));
    serde_(&mut g, "seaport", include_str!("../tests/abi/Seaport.json"));
    serde_(&mut g, "console", include_str!("../tests/abi/console.json"));
}

fn serde_(g: &mut BenchmarkGroup<'_, WallTime>, name: &str, s: &str) {
    type A = alloy_json_abi::JsonAbi;
    type E = ethabi::Contract;

    g.bench_function(format!("{name}/ser/alloy"), |b| {
        let abi = serde_json::from_str::<A>(s).unwrap();
        b.iter(|| serde_json::to_string(black_box(&abi)).unwrap());
    });
    g.bench_function(format!("{name}/ser/ethabi"), |b| {
        let abi = serde_json::from_str::<E>(s).unwrap();
        b.iter(|| serde_json::to_string(black_box(&abi)).unwrap());
    });

    g.bench_function(format!("{name}/de/alloy"), |b| {
        b.iter(|| -> A { serde_json::from_str(black_box(s)).unwrap() });
    });
    g.bench_function(format!("{name}/de/ethabi"), |b| {
        b.iter(|| -> E { serde_json::from_str(black_box(s)).unwrap() });
    });
}

fn signature(c: &mut Criterion) {
    let mut g = c.benchmark_group("signature");
    g.warm_up_time(Duration::from_secs(1));
    signature_(&mut g, "large-function", include_str!("../tests/abi/LargeFunction.json"));
}

fn signature_(g: &mut BenchmarkGroup<'_, WallTime>, name: &str, s: &str) {
    let mut alloy = serde_json::from_str::<alloy_json_abi::Function>(s).unwrap();
    let mut ethabi = serde_json::from_str::<ethabi::Function>(s).unwrap();

    assert_eq!(alloy.selector(), ethabi.short_signature());

    // clear outputs so ethabi doesn't format them
    alloy.outputs.clear();
    ethabi.outputs.clear();

    assert_eq!(alloy.selector(), ethabi.short_signature());
    assert_eq!(alloy.signature(), ethabi.signature());

    g.bench_function(format!("{name}/alloy"), |b| {
        b.iter(|| black_box(&alloy).signature());
    });
    g.bench_function(format!("{name}/ethabi"), |b| {
        b.iter(|| black_box(&ethabi).signature());
    });
}

criterion_group!(benches, serde, signature);
criterion_main!(benches);
