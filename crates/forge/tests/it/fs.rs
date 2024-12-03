//! Filesystem tests.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use foundry_config::{fs_permissions::PathPermission, FsPermissions};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_disabled() {
    let runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fs_permissions = FsPermissions::new(vec![PathPermission::none("./")]);
    });
    let filter = Filter::new(".*", ".*", ".*fs/Disabled");
    TestConfig::with_filter(runner, filter).run().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_default() {
    let runner = TEST_DATA_DEFAULT.runner_with(|config| {
        config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    });
    let filter = Filter::new(".*", ".*", ".*fs/Default");
    TestConfig::with_filter(runner, filter).run().await;
}
