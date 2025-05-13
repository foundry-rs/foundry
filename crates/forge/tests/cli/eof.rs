// Ensure we can build and decode EOF bytecode.
forgetest_init!(test_build_with_eof, |prj, cmd| {
    cmd.forge_fuse()
        .args(["build", "src/Counter.sol", "--eof", "--use", "0.8.29"])
        .assert_success()
        .stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

"#]]);
});

// Ensure compiler fails if doesn't support EOFs but eof flag used.
forgetest_init!(test_unsupported_compiler, |prj, cmd| {
    cmd.forge_fuse()
        .args(["build", "src/Counter.sol", "--eof", "--use", "0.8.27"])
        .assert_failure()
        .stderr_eq(str![[r#"
...
Error: Compiler run failed:
...

"#]]);
});
