use crate::constants::*;
// checks that `clean` works
forgetest_init!(can_clean_config, |prj, cmd| {
    // Resolc does not respect the `out` settings, example:
    // prj.update_config(|config| config.out = "custom-out".into());
    cmd.args(["build", "--resolc"]).assert_success();

    let artifact = prj.root().join(format!("resolc-out/{TEMPLATE_TEST_CONTRACT_ARTIFACT_JSON}"));
    assert!(artifact.exists());

    cmd.forge_fuse().arg("clean").assert_empty_stdout();
    assert!(!artifact.exists());
});

forgetest!(must_rebuild_when_used_the_same_out, |prj, cmd| {
    prj.add_raw_source(
        "Foo",
        r"
pragma solidity *;
contract Foo {}
   ",
    )
    .unwrap();

    // compile with solc
    cmd.args(["build", "--out=resolc-out"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    let artifact = prj.root().join("resolc-out/");
    assert!(artifact.exists());

    // compile with resolc to the same output dir (resolc has hardcoded output dir)
    cmd.forge_fuse().args(["build", "--resolc"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [RESOLC_VERSION]
[RESOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);

    // compile again with solc to the same output dir
    cmd.forge_fuse().args(["build", "--out=resolc-out"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});
