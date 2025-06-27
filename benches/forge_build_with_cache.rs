use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use foundry_bench::{
    get_benchmark_versions, switch_foundry_version, BenchmarkProject, BENCHMARK_REPOS, SAMPLE_SIZE,
};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

fn benchmark_forge_build_with_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("forge-build-with-cache");
    group.sample_size(SAMPLE_SIZE);

    // Setup all projects once - clone repos in parallel
    let projects: Vec<_> = BENCHMARK_REPOS
        .par_iter()
        .map(|repo_config| {
            let project = BenchmarkProject::setup(repo_config).expect("Failed to setup project");
            (repo_config, project)
        })
        .collect();

    // Get versions from environment variable or default
    let versions = get_benchmark_versions();

    for version in versions {
        // Switch foundry version once per version
        switch_foundry_version(&version).expect("Failed to switch foundry version");

        projects.par_iter().for_each(|(_repo_config, project)| {
            let _ = project.run_forge_build(false);
        });

        // Run benchmarks for each project
        for (repo_config, project) in &projects {
            // Format: table_name/column_name/row_name
            // This creates: forge-build-with-cache/{version}/{repo_name}
            let bench_id = BenchmarkId::new(&version, repo_config.name);
            group.bench_function(bench_id, |b| {
                b.iter(|| {
                    println!("Benching: forge-build-with-cache/{}/{}", version, repo_config.name);
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
