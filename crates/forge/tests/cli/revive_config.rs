//! Contains tests for checking configuration for revive

use foundry_test_utils::util::OTHER_SOLC_VERSION;

pub const OTHER_REVIVE_VERSION: &str = "revive:0.1.0-dev.13";

// tests that `--use-revive <revive>` works
forgetest!(can_use_revive, |prj, cmd| {
    prj.add_raw_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();

    cmd.args(["build", "--use", OTHER_SOLC_VERSION, "--revive"]).assert_success().stdout_eq(str![
        [r#"
[COMPILING_FILES] with [REVIVE_VERSION]
[REVIVE_VERSION] [ELAPSED]
Compiler run successful!

"#]
    ]);

    cmd.forge_fuse().args([
        "build",
        "--force",
        "--revive",
        "--use-revive",
        &format!("revive:{OTHER_REVIVE_VERSION}"),
    ]);
    cmd.assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [REVIVE_VERSION]
[REVIVE_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // fails to use resolc that does not exist
    cmd.forge_fuse().args(["build", "--revive", "--use-revive", "this/resolc/does/not/exist"]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: `revive` this/resolc/does/not/exist does not exist

"#]]);
});
