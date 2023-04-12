//! Tests for reproducing issues

use forge::test_utils::{
    config::*,
    test_helpers::{filter::Filter, PROJECT},
};
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};

#[test]
fn test_fs_disabled() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::none("./")]);
    let runner = runner_with_config(config);
    let filter = Filter::new(".*", ".*", ".*fs/Disabled");
    TestConfig::with_filter(runner, filter).run();
}
#[test]
fn test_fs_default() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read("./fixtures")]);
    let runner = runner_with_config(config);
    let filter = Filter::new(".*", ".*", ".*fs/Default");
    TestConfig::with_filter(runner, filter).run();
}
