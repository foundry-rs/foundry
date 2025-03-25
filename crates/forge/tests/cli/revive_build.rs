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
