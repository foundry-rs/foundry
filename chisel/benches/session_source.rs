use chisel::session_source::{SessionSource, SessionSourceConfig};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ethers_solc::Solc;
use forge::executor::opts::EvmOpts;
use foundry_config::Config;

/// Benchmark for the `clone_with_new_line` function in [SessionSource]
fn clone_with_new_line(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    // Grab an empty session source
    let session_source = get_empty_session_source();
    g.bench_function("clone_with_new_line", |b| {
        b.iter(|| black_box(|| session_source.clone_with_new_line("uint a = 1".to_owned())))
    });
}

/// Benchmark for the `build` function in [SessionSource]
fn build(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    g.bench_function("build", |b| {
        b.iter(|| {
            // Grab an empty session source
            let mut session_source = get_empty_session_source();
            black_box(move || session_source.build().unwrap())
        })
    });
}

/// Benchmark for the `execute` function in [SessionSource]
fn execute(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    g.bench_function("execute", |b| {
        b.iter(|| {
            // Grab an empty session source
            let mut session_source = get_empty_session_source();
            black_box(async move { session_source.execute().await.unwrap() })
        })
    });
}

/// Benchmark for the `inspect` function in [SessionSource]
fn inspect(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    g.bench_function("inspect", |b| {
        b.iter(|| {
            // Grab an empty session source
            let mut session_source = get_empty_session_source();
            // Add a uint named "a" with value 1 to the session source
            session_source.with_run_code("uint a = 1");
            black_box(async move { session_source.inspect("a").await.unwrap() })
        })
    });
}

/// Helper function for getting an empty [SessionSource] with default configuration
fn get_empty_session_source() -> SessionSource {
    let solc = Solc::find_or_install_svm_version("0.8.17").unwrap();
    SessionSource::new(
        solc,
        SessionSourceConfig {
            foundry_config: Config::default(),
            evm_opts: EvmOpts::default(),
            backend: None,
            traces: false,
        },
    )
}

criterion_group!(session_source_benches, clone_with_new_line, build, execute, inspect);
criterion_main!(session_source_benches);
