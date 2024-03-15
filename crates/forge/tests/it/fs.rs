//! Filesystem tests.

use crate::{config::*, test_helpers::TEST_DATA_DEFAULT};
use foundry_config::{fs_permissions::PathPermission, FsPermissions};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_disabled() {
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::none("./")]);
    let runner = runner_with_config(&TEST_DATA_DEFAULT, config);
    let filter = Filter::new(".*", ".*", ".*fs/Disabled");
    TestConfig::with_filter(runner, filter).run().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_default() {
    let mut config = TEST_DATA_DEFAULT.config.clone();
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = runner_with_config(&TEST_DATA_DEFAULT, config);
    let filter = Filter::new(".*", ".*", ".*fs/Default");
    TestConfig::with_filter(runner, filter).run().await;
}
