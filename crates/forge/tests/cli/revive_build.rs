use crate::utils::generate_large_init_contract;

forgetest_init!(can_build_with_revive, |prj, cmd| {
    cmd.args(["build", "--revive-compile"]).assert_success();
});

forgetest_init!(force_buid_with_revive, |prj, cmd| {
    cmd.args(["build", "--revive-compile", "--force"]).assert_success();
});

forgetest!(initcode_size_limit_can_be_ignored_revive, |prj, cmd| {
    // This test won't work until this issue is fixed:https://github.com/paritytech/revive/issues/172
    prj.add_source("LargeContract", generate_large_init_contract(249_000).as_str()).unwrap();
    cmd.args(["build", "--revive-compile", "--sizes", "--ignore-eip-3860"]).assert_success();
});
