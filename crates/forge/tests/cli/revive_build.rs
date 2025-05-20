use foundry_test_utils::snapbox::IntoData;

use crate::utils::generate_large_init_contract;
pub const OTHER_RESOLC_VERSION: &str = "0.1.0-dev.13";
pub const NEWEST_RESOLC_VERSION: &str = "0.1.0-dev.16";

forgetest_init!(can_build_with_resolc, |prj, cmd| {
    cmd.args(["build", "--resolc-compile"]).assert_success();
});

forgetest_init!(force_buid_with_resolc, |prj, cmd| {
    cmd.args(["build", "--resolc-compile", "--force"]).assert_success();
});

forgetest!(code_size_exceeds_limit_with_resolc, |prj, cmd| {
    prj.add_source("LargeContract.sol", generate_large_init_contract(50_000).as_str()).unwrap();
    cmd.args([
        "build",
        "--resolc-compile",
        "--sizes",
        "--use-resolc",
        &format!("resolc:{OTHER_RESOLC_VERSION}"),
    ])
    .assert_failure()
    .stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
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
            "--resolc-compile",
            "--use-resolc",
            &format!("resolc:{OTHER_RESOLC_VERSION}"),
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

forgetest_init!(build_contracts_with_optimization, |prj, cmd| {
    cmd.args([
        "build",
        "--resolc",
        "--resolc-optimization",
        "0",
        "--sizes",
        "--json",
        "--use-resolc",
        &format!("resolc:{NEWEST_RESOLC_VERSION}"),
    ])
    .assert_success()
    .stdout_eq(
        str![[r#"
{
   "Counter" :{
      "runtime_size":11175,
      "init_size":11175,
      "runtime_margin":238825,
      "init_margin":238825
   }
}
"#]]
        .is_json(),
    );

    cmd.forge_fuse()
        .args([
            "build",
            "--resolc",
            "--resolc-optimization",
            "z",
            "--sizes",
            "--json",
            "--use-resolc",
            &format!("resolc:{NEWEST_RESOLC_VERSION}"),
        ])
        .assert_success()
        .stdout_eq(
            str![[r#"
{
   "Counter" :{
      "runtime_size":4994,
      "init_size":4994,
      "runtime_margin":245006,
      "init_margin":245006
   }
}
"#]]
            .is_json(),
        );
});
