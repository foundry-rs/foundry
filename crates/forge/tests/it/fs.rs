//! Filesystem tests.

use crate::{config::*, test_helpers::PROJECT};
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};
use foundry_test_utils::Filter;

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_disabled() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::none("./")]);
    let runner = runner_with_config(config).await;
    let filter = Filter::new(".*", ".*", ".*fs/Disabled");
    TestConfig::with_filter(runner, filter).run().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_default() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = runner_with_config(config);
    let filter = Filter::new(".*", ".*", ".*fs/Default");
    TestConfig::with_filter(runner.await, filter).run().await;
}
