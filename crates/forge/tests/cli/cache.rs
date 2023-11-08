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
