use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::{setup_benchmark_repos, SAMPLE_SIZE};
use foundry_common::sh_println;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::env;

fn benchmark_forge_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("forge-test");
    group.sample_size(SAMPLE_SIZE);

    // Get the current version being tested
    let version =
        env::var("FOUNDRY_BENCH_CURRENT_VERSION").unwrap_or_else(|_| "unknown".to_string());

    let _ = sh_println!("Running forge-test for version: {version}");

    let projects = setup_benchmark_repos();

    projects.par_iter().for_each(|(_repo_config, project)| {
        project.run_forge_build(false).expect("forge build failed");
    });

    for (repo_config, project) in &projects {
        // This creates: forge-test/{version}/{repo_name}
        let bench_id = BenchmarkId::new(&version, &repo_config.name);

        group.bench_function(bench_id, |b| {
            b.iter(|| {
                let _output = project.run_forge_test().expect("forge test failed");
            });
        });
    }

    group.finish();
}

criterion_group!(benches, benchmark_forge_test);
criterion_main!(benches);
