//! Tests for various cache command.

#[forgetest]
fn can_list_cache(cmd: _) {
    cmd.args(["cache", "ls"]);
    cmd.assert_success();
}

#[forgetest]
fn can_list_cache_all(cmd: _) {
    cmd.args(["cache", "ls", "all"]);
    cmd.assert_success();
}

#[forgetest]
fn can_list_specific_chain(cmd: _) {
    cmd.args(["cache", "ls", "mainnet"]);
    cmd.assert_success();
}

#[forgetest]
fn cache_ls_output_on_stderr(cmd: _) {
    cmd.args(["cache", "ls", "mainnet"]).assert_success().stdout_eq(str![""]);
}

#[forgetest_init]
fn can_test_no_cache(prj: _, cmd: _) {
    prj.initialize_default_contracts();
    prj.clear_cache();

    cmd.args(["test", "--no-cache"]).assert_success();
    assert!(!prj.cache().exists(), "cache file should not exist");

    cmd.forge_fuse().arg("test").assert_success();
    assert!(prj.cache().exists(), "cache file should exist");
}
