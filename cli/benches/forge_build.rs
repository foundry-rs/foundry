use criterion::{criterion_group, criterion_main, Criterion};
use foundry_cli_test_utils;

// TODO helper functions for checkout + build


fn forge_test_benchmark(c: &mut Criterion) {

    let mut group = c.benchmark_group("forge test solmate");
    // group.bench_function("solmate", |b| b.iter(|| {
    //
    // TODO: run test via cmd here
    //
    // } ));
}

criterion_group!(benches, forge_test_benchmark);
criterion_main!(benches);