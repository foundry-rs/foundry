//! Tests for reproducing issues

use crate::{
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
