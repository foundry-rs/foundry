use chisel::session_source::{SessionSource, SessionSourceConfig};
use criterion::{criterion_group, Criterion};
use foundry_compilers::solc::Solc;
use semver::Version;
use std::{hint::black_box, sync::LazyLock};
use tokio::runtime::Runtime;

static SOLC: LazyLock<Solc> =
    LazyLock::new(|| Solc::find_or_install(&Version::new(0, 8, 19)).unwrap());

/// Benchmark for the `clone_with_new_line` function in [SessionSource]
fn clone_with_new_line(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    // Grab an empty session source
    g.bench_function("clone_with_new_line", |b| {
        b.iter(|| {
            let session_source = get_empty_session_source();
            let new_line = String::from("uint a = 1");
            black_box(session_source.clone_with_new_line(new_line).unwrap());
        })
    });
}

/// Benchmark for the `build` function in [SessionSource]
fn build(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    g.bench_function("build", |b| {
        b.iter(|| {
            // Grab an empty session source
            let mut session_source = get_empty_session_source();
            black_box(session_source.build().unwrap())
        })
    });
}

/// Benchmark for the `execute` function in [SessionSource]
fn execute(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    g.bench_function("execute", |b| {
        b.to_async(rt()).iter(|| async {
            // Grab an empty session source
            let mut session_source = get_empty_session_source();
            black_box(session_source.execute().await.unwrap())
        })
    });
}

/// Benchmark for the `inspect` function in [SessionSource]
fn inspect(c: &mut Criterion) {
    let mut g = c.benchmark_group("session_source");

    g.bench_function("inspect", |b| {
        b.to_async(rt()).iter(|| async {
            // Grab an empty session source
            let mut session_source = get_empty_session_source();
            // Add a uint named "a" with value 1 to the session source
            session_source.with_run_code("uint a = 1");
            black_box(session_source.inspect("a").await.unwrap())
        })
    });
}

/// Helper function for getting an empty [SessionSource] with default configuration
fn get_empty_session_source() -> SessionSource {
    SessionSource::new(SOLC.clone(), SessionSourceConfig::default())
}

fn rt() -> Runtime {
    Runtime::new().unwrap()
}

fn main() {
    // Install before benches if not present
    let _ = LazyLock::force(&SOLC);

    session_source_benches();

    Criterion::default().configure_from_args().final_summary()
}

criterion_group!(session_source_benches, clone_with_new_line, build, execute, inspect);
