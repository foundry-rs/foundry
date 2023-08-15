//! Tests for various cache command
use foundry_tests::{
    forgetest,
    util::{TestCommand, TestProject},
};

forgetest!(can_list_cache, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["cache", "ls"]);
    cmd.assert_success();
});

forgetest!(can_list_cache_all, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["cache", "ls", "all"]);
    cmd.assert_success();
});

forgetest!(can_list_specific_chain, |_prj: TestProject, mut cmd: TestCommand| {
    cmd.args(["cache", "ls", "mainnet"]);
    cmd.assert_success();
});
