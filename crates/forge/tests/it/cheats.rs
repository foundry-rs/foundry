//! Forge tests for cheatcodes.

use crate::{
    config::*,
    test_helpers::{
        ForgeTestData, RE_PATH_SEPARATOR, TEST_DATA_CANCUN, TEST_DATA_DEFAULT,
        TEST_DATA_MULTI_VERSION,
    },
};
use alloy_primitives::U256;
use foundry_config::{fs_permissions::PathPermission, FsPermissions};
use foundry_test_utils::Filter;

/// Executes all cheat code tests but not fork cheat codes or tests that require isolation mode or
/// specific seed.
async fn test_cheats_local(test_data: &ForgeTestData) {
    let mut filter = Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}*"))
        .exclude_paths("Fork")
        .exclude_contracts("(Isolated|WithSeed)");

    // Exclude FFI tests on Windows because no `echo`, and file tests that expect certain file paths
    if cfg!(windows) {
        filter = filter.exclude_tests("(Ffi|File|Line|Root)");
    }

    if cfg!(feature = "isolate-by-default") {
        filter = filter.exclude_contracts("(LastCallGasDefaultTest|MockFunctionTest|WithSeed)");
    }

    let mut config = test_data.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read_write("./")]);
    let runner = test_data.runner_with_config(config);

    TestConfig::with_filter(runner, filter).run().await;
}

/// Executes subset of all cheat code tests in isolation mode.
async fn test_cheats_local_isolated(test_data: &ForgeTestData) {
    let filter = Filter::new(".*", ".*(Isolated)", &format!(".*cheats{RE_PATH_SEPARATOR}*"));

    let mut config = test_data.config.clone();
    config.isolate = true;
    let runner = test_data.runner_with_config(config);

    TestConfig::with_filter(runner, filter).run().await;
}

/// Executes subset of all cheat code tests using a specific seed.
async fn test_cheats_local_with_seed(test_data: &ForgeTestData) {
    let filter = Filter::new(".*", ".*(WithSeed)", &format!(".*cheats{RE_PATH_SEPARATOR}*"));

    let mut config = test_data.config.clone();
    config.fuzz.seed = Some(U256::from(100));
    let runner = test_data.runner_with_config(config);

    TestConfig::with_filter(runner, filter).run().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_default() {
    test_cheats_local(&TEST_DATA_DEFAULT).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_default_isolated() {
    test_cheats_local_isolated(&TEST_DATA_DEFAULT).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_default_with_seed() {
    test_cheats_local_with_seed(&TEST_DATA_DEFAULT).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_multi_version() {
    test_cheats_local(&TEST_DATA_MULTI_VERSION).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_cancun() {
    test_cheats_local(&TEST_DATA_CANCUN).await
}
