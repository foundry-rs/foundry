#![allow(unknown_lints, clippy::incompatible_msrv, missing_docs)]

use alloy_primitives::{keccak256, Address, B256};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn primitives(c: &mut Criterion) {
    let mut g = c.benchmark_group("primitives");
    g.bench_function("address/checksum", |b| {
        let address = Address::random();
        let out = &mut [0u8; 42];
        b.iter(|| {
            let x = address.to_checksum_raw(black_box(out), None);
            black_box(x);
        })
    });
    g.bench_function("keccak256/32", |b| {
        let mut out = B256::random();
        b.iter(|| {
            out = keccak256(out.as_slice());
            black_box(&out);
        });
    });
    g.finish();
}

criterion_group!(benches, primitives);
criterion_main!(benches);
