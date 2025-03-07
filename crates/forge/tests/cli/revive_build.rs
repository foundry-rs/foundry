use crate::utils::generate_large_init_contract;
use foundry_compilers::artifacts::BytecodeHash;
use foundry_config::Config;

forgetest_init!(can_build_with_revive, |prj, cmd| {
    // BytecodeHash issue workaround https://github.com/paritytech/revive/issues/219
    prj.write_config(Config { bytecode_hash: BytecodeHash::None, ..Default::default() });
    cmd.args(["build", "--revive-compile"]).assert_success();
});

forgetest_init!(force_buid_with_revive, |prj, cmd| {
    // BytecodeHash issue workaround https://github.com/paritytech/revive/issues/219
    prj.write_config(Config { bytecode_hash: BytecodeHash::None, ..Default::default() });
    cmd.args(["build", "--revive-compile", "--force"]).assert_success();
});

forgetest!(initcode_size_limit_can_be_ignored_revive, |prj, cmd| {
    // BytecodeHash issue workaround https://github.com/paritytech/revive/issues/219
    prj.write_config(Config { bytecode_hash: BytecodeHash::None, ..Default::default() });
    // This test won't work until this issue is fixed:https://github.com/paritytech/revive/issues/172
    prj.add_source("LargeContract", generate_large_init_contract(249_000).as_str()).unwrap();
    cmd.args(["build", "--revive-compile", "--sizes", "--ignore-eip-3860"]).assert_success();
});
