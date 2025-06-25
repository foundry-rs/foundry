use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::{switch_foundry_version, BenchmarkProject, BENCHMARK_REPOS, FOUNDRY_VERSIONS};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
fn benchmark_forge_test(c: &mut Criterion) {
    let mut group = c.benchmark_group("forge-test");
    group.sample_size(10);

    // Setup all projects once - clone repos in parallel
    let projects: Vec<_> = BENCHMARK_REPOS
        .par_iter()
        .map(|repo_config| {
            // Setup: prepare project (clone repo)
            let project = BenchmarkProject::setup(repo_config).expect("Failed to setup project");
            (repo_config, project)
        })
        .collect();

    for &version in FOUNDRY_VERSIONS {
        // Switch foundry version once per version
        switch_foundry_version(version).expect("Failed to switch foundry version");

        // Build all projects in parallel for this foundry version
        projects.par_iter().for_each(|(_repo_config, project)| {
            project.run_forge_build(false).expect("forge build failed");
        });

        // Run benchmarks for each project
        for (repo_config, project) in &projects {
            // Format: table_name/column_name/row_name
            // This creates: forge-test/{version}/{repo_name}
            let bench_id = BenchmarkId::new(version, repo_config.name);

            group.bench_function(bench_id, |b| {
                b.iter(|| {
                    let output = project.run_forge_test().expect("forge test failed");
                    black_box(output);
                });
            });
        }
    }

    group.finish();
}

criterion_group!(benches, benchmark_forge_test);
criterion_main!(benches);
