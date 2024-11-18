//! Forge forking tests.

use crate::{
    config::*,
    test_helpers::{RE_PATH_SEPARATOR, TEST_DATA_DEFAULT},
};
use alloy_chains::Chain;
use forge::result::SuiteResult;
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};
use foundry_test_utils::Filter;
use std::fs;

/// Executes reverting fork test
#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_fork_revert() {
    let filter = Filter::new(
        "testNonExistingContractRevert",
        ".*",
        &format!(".*cheats{RE_PATH_SEPARATOR}Fork"),
    );
    let mut runner = TEST_DATA_DEFAULT.runner();
    let suite_result = runner.test_collect(&filter);
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
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = TEST_DATA_DEFAULT.runner_with_config(config);
    let filter = Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::with_filter(runner, filter).run().await;
}

/// Executes eth_getLogs cheatcode
#[tokio::test(flavor = "multi_thread")]
async fn test_get_logs_fork() {
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = TEST_DATA_DEFAULT.runner_with_config(config);
    let filter = Filter::new("testEthGetLogs", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::with_filter(runner, filter).run().await;
}

/// Executes rpc cheatcode
#[tokio::test(flavor = "multi_thread")]
async fn test_rpc_fork() {
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = TEST_DATA_DEFAULT.runner_with_config(config);
    let filter = Filter::new("testRpc", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
        .exclude_tests(".*Revert");
    TestConfig::with_filter(runner, filter).run().await;
}

/// Tests that we can launch in forking mode
#[tokio::test(flavor = "multi_thread")]
async fn test_launch_fork() {
    let rpc_url = foundry_test_utils::rpc::next_http_archive_rpc_endpoint();
    let runner = TEST_DATA_DEFAULT.forked_runner(&rpc_url).await;
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Launch"));
    TestConfig::with_filter(runner, filter).run().await;
}

/// Smoke test that forking workings with websockets
#[tokio::test(flavor = "multi_thread")]
async fn test_launch_fork_ws() {
    let rpc_url = foundry_test_utils::rpc::next_ws_archive_rpc_endpoint();
    let runner = TEST_DATA_DEFAULT.forked_runner(&rpc_url).await;
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Launch"));
    TestConfig::with_filter(runner, filter).run().await;
}

/// Tests that we can transact transactions in forking mode
#[tokio::test(flavor = "multi_thread")]
async fn test_transact_fork() {
    let runner = TEST_DATA_DEFAULT.runner();
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}Transact"));
    TestConfig::with_filter(runner, filter).run().await;
}

/// Tests that we can create the same fork (provider,block) concurrently in different tests
#[tokio::test(flavor = "multi_thread")]
async fn test_create_same_fork() {
    let runner = TEST_DATA_DEFAULT.runner();
    let filter = Filter::new(".*", ".*", &format!(".*fork{RE_PATH_SEPARATOR}ForkSame"));
    TestConfig::with_filter(runner, filter).run().await;
}

/// Test that `no_storage_caching` config is properly applied
#[tokio::test(flavor = "multi_thread")]
async fn test_storage_caching_config() {
    // no_storage_caching set to true: storage should not be cached
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.no_storage_caching = true;
    let runner = TEST_DATA_DEFAULT.runner_with_config(config);
    let filter =
        Filter::new("testStorageCaching", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
            .exclude_tests(".*Revert");
    TestConfig::with_filter(runner, filter).run().await;
    let cache_dir = Config::foundry_block_cache_dir(Chain::mainnet(), 19800000).unwrap();
    let _ = fs::remove_file(cache_dir);

    // no_storage_caching set to false: storage should be cached
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.no_storage_caching = false;
    let runner = TEST_DATA_DEFAULT.runner_with_config(config);
    let filter =
        Filter::new("testStorageCaching", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}Fork"))
            .exclude_tests(".*Revert");
    TestConfig::with_filter(runner, filter).run().await;
    let cache_dir = Config::foundry_block_cache_dir(Chain::mainnet(), 19800000).unwrap();
    assert!(cache_dir.exists());

    // cleanup cached storage so subsequent tests does not fail
    let _ = fs::remove_file(cache_dir);
}
