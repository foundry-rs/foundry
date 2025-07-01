use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::{setup_benchmark_repos, SAMPLE_SIZE};
use foundry_common::sh_println;
use std::env;

fn benchmark_forge_build_no_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("forge-build-no-cache");
    group.sample_size(SAMPLE_SIZE);

    // Get the current version being tested
    let version =
        env::var("FOUNDRY_BENCH_CURRENT_VERSION").unwrap_or_else(|_| "unknown".to_string());

    let _ = sh_println!("Running forge-build-no-cache for version: {version}");

    let projects = setup_benchmark_repos();

    for (repo_config, project) in &projects {
        // This creates: forge-build-no-cache/{version}/{repo_name}
        let bench_id = BenchmarkId::new(&version, &repo_config.name);

        group.bench_function(bench_id, |b| {
            b.iter(|| {
                let _output = project.run_forge_build(true).expect("forge build failed");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_forge_build_no_cache);
criterion_main!(benches);
