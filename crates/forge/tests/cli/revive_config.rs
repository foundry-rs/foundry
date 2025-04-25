//! Contains tests for checking configuration for resolc

use foundry_test_utils::util::OTHER_SOLC_VERSION;

pub const OTHER_RESOLC_VERSION: &str = "resolc:0.1.0-dev.13";

// tests that `--use-resolc <resolc>` works
forgetest!(can_use_resolc, |prj, cmd| {
    prj.add_raw_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();

    cmd.args(["build", "--use", OTHER_SOLC_VERSION, "--resolc"]).assert_success().stdout_eq(str![
        [r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]
    ]);

    cmd.forge_fuse().args([
        "build",
        "--force",
        "--resolc",
        "--use-resolc",
        &format!("resolc:{OTHER_RESOLC_VERSION}"),
    ]);

    cmd.assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // fails to use resolc that does not exist
    cmd.forge_fuse().args(["build", "--resolc", "--use-resolc", "this/resolc/does/not/exist"]);
    cmd.assert_failure().stderr_eq(str![[r#"
Error: `resolc` this/resolc/does/not/exist does not exist

"#]]);
});
