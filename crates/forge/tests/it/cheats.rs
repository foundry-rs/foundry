//! Forge tests for cheatcodes.

use crate::{
    config::*,
    test_helpers::{RE_PATH_SEPARATOR, TEST_DATA_DEFAULT},
};
use foundry_config::{fs_permissions::PathPermission, FsPermissions};
use foundry_test_utils::Filter;

/// Executes all cheat code tests but not fork cheat codes
#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local() {
    let mut filter =
        Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}*")).exclude_paths("Fork");

    // Exclude FFI tests on Windows because no `echo`, and file tests that expect certain file paths
    if cfg!(windows) {
        filter = filter.exclude_tests("(Ffi|File|Line|Root)");
    }

    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read_write("./")]);
    let runner = TEST_DATA_DEFAULT.runner_with_config(config, false);

    TestConfig::with_filter(runner, filter).run().await;
}
