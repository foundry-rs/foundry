use criterion::{criterion_group, criterion_main, Criterion};
use foundry_test_utils::{
    util::{lossy_string, setup_forge_remote},
    TestCommand, TestProject,
};

/// Returns a cloned and `forge built` `solmate` project
fn built_solmate() -> (TestProject, TestCommand) {
    setup_forge_remote("transmissions11/solmate")
}

fn forge_test_benchmark(c: &mut Criterion) {
    let (prj, _) = built_solmate();

    let mut group = c.benchmark_group("forge test");
    group.sample_size(10);
    group.bench_function("solmate", |b| {
        let mut cmd = prj.forge_command();
        cmd.arg("test");
        b.iter(|| {
            let output = cmd.execute();
            println!("stdout:\n{}", lossy_string(&output.stdout));
            println!("\nstderr:\n{}", lossy_string(&output.stderr));
        });
    });
}

criterion_group!(benches, forge_test_benchmark);
criterion_main!(benches);
