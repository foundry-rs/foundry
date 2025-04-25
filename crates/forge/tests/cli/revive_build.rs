use foundry_test_utils::snapbox::IntoData;

use crate::utils::generate_large_init_contract;
pub const OTHER_REVIVE_VERSION: &str = "revive:0.1.0-dev.13";

forgetest_init!(can_build_with_revive, |prj, cmd| {
    cmd.args(["build", "--revive-compile"]).assert_success();
});

forgetest_init!(force_buid_with_revive, |prj, cmd| {
    cmd.args(["build", "--revive-compile", "--force"]).assert_success();
});

forgetest!(code_size_exceeds_limit_with_revive, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str()).unwrap();
    cmd.args([
        "build",
        "--revive-compile",
        "--sizes",
        "--use-revive",
        &format!("revive:{OTHER_REVIVE_VERSION}"),
    ])
    .assert_failure()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [REVIVE_VERSION]
[REVIVE_VERSION] [ELAPSED]
Compiler run successful!

╭---------------+------------------+-------------------+--------------------+---------------------╮
| Contract      | Runtime Size (B) | Initcode Size (B) | Runtime Margin (B) | Initcode Margin (B) |
+=================================================================================================+
| LargeContract | 264,700          | 264,700           | -14,700            | -14,700             |
╰---------------+------------------+-------------------+--------------------+---------------------╯


"#]]);

    cmd.forge_fuse()
        .args([
            "build",
            "--revive-compile",
            "--use-revive",
            &format!("revive:{OTHER_REVIVE_VERSION}"),
            "--sizes",
            "--json",
        ])
        .assert_failure()
        .stdout_eq(
            str![[r#"
{
  "LargeContract": {
    "runtime_size": 264700,
    "init_size": 264700,
    "runtime_margin": -14700,
    "init_margin": -14700
  }
}
"#]]
            .is_json(),
        );
});
