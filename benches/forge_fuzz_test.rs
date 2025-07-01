use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::setup_benchmark_repos;
use foundry_common::sh_println;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::env;

pub fn forge_fuzz_test_benchmark(c: &mut Criterion) {
    // Get the version to test from environment variable
    let version =
        env::var("FOUNDRY_BENCH_CURRENT_VERSION").unwrap_or_else(|_| "unknown".to_string());

    let mut group = c.benchmark_group("forge-fuzz-test");
    group.sample_size(foundry_bench::SAMPLE_SIZE);

    let _ = sh_println!("Running forge-fuzz-test for version: {}", version);

    let projects = setup_benchmark_repos();

    projects.par_iter().for_each(|(_repo_config, project)| {
        project.run_forge_build(false).expect("forge build failed");
    });

    for (repo_config, project) in &projects {
        // This creates: forge-fuzz-test/{version}/{repo_name}
        let bench_id = BenchmarkId::new(&version, &repo_config.name);

        group.bench_function(bench_id, |b| {
            b.iter(|| {
                let _output = project.run_fuzz_tests().expect("forge fuzz test failed");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, forge_fuzz_test_benchmark);
criterion_main!(benches);
