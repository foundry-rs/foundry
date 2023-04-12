use criterion::criterion_main;

mod benchmarks;

criterion_main! {
    benchmarks::fuzz::benches,
    benchmarks::invariant::benches,
}