//! forge tests for cheat codes

use crate::{
    config::*,
    test_helpers::{filter::Filter, PROJECT, RE_PATH_SEPARATOR},
};
use forge::result::SuiteResult;
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};

/// Executes reverting fork test
#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_fork_revert() {
    let mut runner = runner().await;
    let suite_result = runner
        .test(
            &Filter::new(
                "testNonExistingContractRevert",
                ".*",
                &format!(".*cheats{RE_PATH_SEPARATOR}Fork"),
            ),
            None,
            test_opts(),
        )
        .await;
    assert_eq!(suite_result.len(), 1);

    for (_, SuiteResult { test_results, .. }) in suite_result {
        for (_, result) in test_results {
            assert_eq!(
                result.reason.unwrap(),
                "Contract 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f does not exist on active fork with id `1`\n        But exists on non active forks: `[0]`"
            );
        }
    }
}

/// Executes all non-reverting fork cheatcodes
#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_fork() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = runner_with_config(config);
    let filter = Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::with_filter(runner.await, filter).run().await;
}

/// Executes eth_getLogs cheatcode
#[tokio::test(flavor = "multi_thread")]
async fn test_get_logs_fork() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = runner_with_config(config);
    let filter = Filter::new("testEthGetLogs", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::with_filter(runner.await, filter).run().await;
}

/// Executes rpc cheatcode
#[tokio::test(flavor = "multi_thread")]
async fn test_rpc_fork() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = runner_with_config(config);
    let filter = Filter::new("testRpc", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::with_filter(runner.await, filter).run().await;
}

/// Tests that we can launch in forking mode
#[tokio::test(flavor = "multi_thread")]
async fn test_launch_fork() {
    let rpc_url = foundry_utils::rpc::next_http_archive_rpc_endpoint();
    let runner = forked_runner(&rpc_url).await;
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Launch"));
    TestConfig::with_filter(runner, filter).run().await;
}

/// Smoke test that forking workings with websockets
#[tokio::test(flavor = "multi_thread")]
async fn test_launch_fork_ws() {
    let rpc_url = foundry_utils::rpc::next_ws_archive_rpc_endpoint();
    let runner = forked_runner(&rpc_url).await;
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Launch"));
    TestConfig::with_filter(runner, filter).run().await;
}

/// Tests that we can transact transactions in forking mode
#[tokio::test(flavor = "multi_thread")]
async fn test_transact_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Transact"));
    TestConfig::filter(filter).await.run().await;
}

/// Tests that we can create the same fork (provider,block) concurretnly in different tests
#[tokio::test(flavor = "multi_thread")]
async fn test_create_same_fork() {
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}ForkSame"));
    TestConfig::filter(filter).await.run().await;
}
