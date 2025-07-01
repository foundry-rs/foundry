use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::setup_benchmark_repos;
use foundry_common::sh_println;
use std::env;

pub fn forge_coverage_benchmark(c: &mut Criterion) {
    // Get the version to test from environment variable
    let version =
        env::var("FOUNDRY_BENCH_CURRENT_VERSION").unwrap_or_else(|_| "unknown".to_string());

    let mut group = c.benchmark_group("forge-coverage");
    group.sample_size(foundry_bench::SAMPLE_SIZE);

    let _ = sh_println!("Running forge-coverage for version: {}", version);

    let projects = setup_benchmark_repos();

    for (repo_config, project) in &projects {
        // This creates: forge-coverage/{version}/{repo_name}
        let bench_id = BenchmarkId::new(&version, &repo_config.name);

        group.bench_function(bench_id, |b| {
            b.iter(|| {
                let _output = project.run_forge_coverage().expect("forge coverage failed");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, forge_coverage_benchmark);
criterion_main!(benches);
