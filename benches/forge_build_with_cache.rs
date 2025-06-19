use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::{install_foundry_version, BenchmarkProject, BENCHMARK_REPOS, FOUNDRY_VERSIONS};

fn benchmark_forge_build_with_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("forge-build-with-cache");
    group.sample_size(10);

    for &version in FOUNDRY_VERSIONS {
        // Install foundry version once per version
        install_foundry_version(version).expect("Failed to install foundry version");

        for repo_config in BENCHMARK_REPOS {
            // Setup: prepare project OUTSIDE benchmark
            let project = BenchmarkProject::setup(repo_config).expect("Failed to setup project");

            // Prime the cache OUTSIDE benchmark
            let _ = project.run_forge_build(false);

            // Format: table_name/column_name/row_name
            // This creates: forge-build-with-cache/{version}/{repo_name}
            let bench_id = BenchmarkId::new(version, repo_config.name);

            group.bench_function(bench_id, |b| {
                b.iter(|| {
                    let output = project.run_forge_build(false).expect("forge build failed");
                    black_box(output);
                });
            });
        }
    }

    group.finish();
}

criterion_group!(benches, benchmark_forge_build_with_cache);
criterion_main!(benches);
