//! Forge tests for cheatcodes.

use crate::{
    config::*,
    test_helpers::{PROJECT, RE_PATH_SEPARATOR},
};
use foundry_compilers::EvmVersion;
use foundry_config::{fs_permissions::PathPermission, Config, FsPermissions};
use foundry_test_utils::Filter;

/// Executes all cheat code tests except fork cheat codes
#[tokio::test(flavor = "multi_thread")]
async fn test_cheats_local() {
    let mut config = Config::with_root(PROJECT.root());
    config.fs_permissions = FsPermissions::new(vec![PathPermission::read_write("./")]);
    // todo: shuffle files around in `testdata` to have multiple projects, and separate cancun out into a new project
    // after that, remove this line
    config.evm_version = EvmVersion::Cancun;
    let runner = runner_with_config(config);
    let filter =
        Filter::new(".*", ".*", &format!(".*cheats{RE_PATH_SEPARATOR}*")).exclude_paths("Fork");

    // on windows exclude ffi tests since no echo and file test that expect a certain file path
    #[cfg(windows)]
    let filter = filter.exclude_tests("(Ffi|File|Line|Root)");

    TestConfig::with_filter(runner.await, filter).run().await;
}
