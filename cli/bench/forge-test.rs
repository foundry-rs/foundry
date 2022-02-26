use criterion::{
    criterion_group, criterion_main, measurement::Measurement, BenchmarkGroup, Criterion,
};
use ethers::solc::PathStyle;
use foundry_cli_test_utils::{
    util::{clone_remote, setup},
    TestCommand,
};

use std::process::{Command, Stdio};

fn setup_repo(repo: &str) -> TestCommand {
    let (prj, mut cmd) = setup(repo, PathStyle::Dapptools);

    // if it exists, forge clean -> forge build to ensure latest artifacts
    if prj.root().exists() {
        cmd.arg("clean");
        cmd.assert_non_empty_stdout();

        cmd.arg("build");
        cmd.assert_non_empty_stdout();
    } else {
        // Wipe the default structure
        prj.wipe();

        // otherwise clone it
        let git_clone = clone_remote(&format!("https://github.com/{}", repo), prj.root());
        assert!(git_clone, "could not clone repository");
    }

    // We just run make install, but we do not care if it worked or not,
    // since some repositories do not have that target
    let _ = Command::new("make")
        .arg("install")
        .current_dir(prj.root())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    cmd
}

fn bench_repo(mut cmd: TestCommand, fork_block: u64, eth_rpc_url: Option<&str>, args: &[&str]) {
    // Skip fork tests if the RPC url is not set.
    if fork_block > 0 && eth_rpc_url.is_none() {
        eprintln!("Skipping test {}. ETH_RPC_URL is not set.", cmd.project().root().display());
        return
    };

    // Run the tests
    cmd.arg("test").args(args).args(["--optimize", "--optimize-runs", "20000", "--ffi"]);

    // cmd.set_env("FOUNDRY_FUZZ_RUNS", "1");
    if fork_block > 0 {
        cmd.set_env("FOUNDRY_ETH_RPC_URL", eth_rpc_url.unwrap());
        cmd.set_env("FOUNDRY_FORK_BLOCK_NUMBER", fork_block.to_string());
    }
    cmd.assert_non_empty_stdout();
}

fn bench_one<M: Measurement>(
    group: &mut BenchmarkGroup<M>,
    repo: &str,
    fork_block: u64,
    eth_rpc_url: Option<&str>,
    args: &[&str],
) {
    group.bench_function(repo, |b| {
        // this is expensive, do once
        setup_repo(repo);
        // calling `setup(..)` is cheap, so we can aford to do inside the
        b.iter(|| bench_repo(setup(repo, PathStyle::Dapptools).1, fork_block, eth_rpc_url, args));
    });
}

fn bench_repos(c: &mut Criterion) {
    let mut group = c.benchmark_group("forge-tests");
    // TODO: Add more.
    bench_one(&mut group, "Rari-Capital/vaults", 0, None, &[]);
}

criterion_group!(forge_test, bench_repos);
criterion_main!(forge_test);
