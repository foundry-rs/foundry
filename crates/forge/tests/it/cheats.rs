//! forge tests for cheat codes

use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};

use crate::{
    config::*,
    test_helpers::{filter::Filter, PROJECT, RE_PATH_SEPARATOR},
};

/// Executes all cheat code tests but not fork cheat codes
#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read_write("./")]);
    let runner = runner_with_config(config);
    let filter =
        Filter::new(".*", "Env*", &format!(".*cheats{RE_PATH_SEPARATOR}*")).exclude_paths("Fork");

    // on windows exclude ffi tests since no echo and file test that expect a certain file path
    #[cfg(windows)]
    let filter = filter.exclude_tests("(Ffi|File|Line|Root)");

    TestConfig::with_filter(runner.await, filter).run().await;
}
