//! Tests for various cache command.

forgetest!(can_list_cache, |_prj, cmd| {
    cmd.args(["cache", "ls"]);
    cmd.assert_success();
});

forgetest!(can_list_cache_all, |_prj, cmd| {
    cmd.args(["cache", "ls", "all"]);
    cmd.assert_success();
});

forgetest!(can_list_specific_chain, |_prj, cmd| {
    cmd.args(["cache", "ls", "mainnet"]);
    cmd.assert_success();
});

forgetest!(can_test_no_cache, |prj, cmd| {
    cmd.args(["test", "--no-cache"]);
    cmd.assert_success();
    assert!(!prj.cache_path().exists(), "cache file should not exist");
    cmd.args(["test"]);
    cmd.assert_success();
    assert!(prj.cache_path().exists(), "cache file should exist");
});
