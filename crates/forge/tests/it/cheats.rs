//! Forge tests for cheatcodes.

use crate::{
    config::*,
    test_helpers::{ForgeTestData, RE_PATH_SEPARATOR, TEST_DATA_DEFAULT, TEST_DATA_MULTI_VERSION},
};
use foundry_config::{fs_permissions::PathPermission, FsPermissions};
use foundry_test_utils::Filter;

/// Executes all cheat code tests but not fork cheat codes
async fn test_cheats_local(test_data: &ForgeTestData) {
    let mut filter =
        Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}*")).exclude_paths("Fork");

    // Exclude FFI tests on Windows because no `echo`, and file tests that expect certain file paths
    if cfg!(windows) {
        filter = filter.exclude_tests("(Ffi|File|Line|Root)");
    }

    let mut config = test_data.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read_write("./")]);
    let runner = test_data.runner_with_config(config);

    TestConfig::with_filter(runner, filter).run().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_default() {
    test_cheats_local(&TEST_DATA_DEFAULT).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local_multi_version() {
    test_cheats_local(&TEST_DATA_MULTI_VERSION).await
}
