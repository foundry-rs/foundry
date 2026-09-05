use foundry_test_utils::util::OutputExt;

forgetest!(can_run_strict_assembly_tests, |prj, cmd| {
    prj.create_file(
        "src/MathUtil.yul",
        r#"
function min(a, b) -> minimum {
    minimum := a
    if lt(b, a) { minimum := b }
}
"#,
    );
    prj.create_file(
        "test/MathUtil.t.yul",
        r#"
import "../src/MathUtil.yul"

function test_min() {
    if iszero(eq(2, min(4, 2))) { revert(0, 0) }
}
"#,
    );

    cmd.args(["test", "--strict-assembly", "--list"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!
test/MathUtil.t.yul
  MathUtil
    test_min


"#]]);

    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/MathUtil.t.yul:MathUtil
[PASS] test_min() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 1 tests passed, 0 failed, 0 skipped (1 total tests)

"#]]);
});

forgetest!(strict_assembly_test_failure_reverts, |prj, cmd| {
    prj.create_file(
        "test/Failure.t.yul",
        r#"
function test_failure() { revert(0, 0) }
"#,
    );

    cmd.args(["test", "--strict-assembly"]).assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/Failure.t.yul:Failure
[FAIL: EvmError: Revert] test_failure() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/Failure.t.yul:Failure
[FAIL: EvmError: Revert] test_failure() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

forgetest!(strict_assembly_splits_suites_and_invalidates_imports, |prj, cmd| {
    prj.create_file("src/Shared.yul", "function shared() -> value { value := 1 }");
    for suite in ["First", "Second"] {
        prj.create_file(
            format!("test/{suite}.t.yul"),
            &format!(
                r#"
import "../src/Shared.yul"
function test_{suite}() {{
    if iszero(eq(shared(), 1)) {{ revert(0, 0) }}
}}
"#
            ),
        );
    }

    let output =
        cmd.args(["test", "--strict-assembly"]).assert_success().get_output().stdout_lossy();
    assert!(output.contains("test_First()"));
    assert!(output.contains("test_Second()"));

    prj.create_file("src/Shared.yul", "function shared() -> value { value := 2 }");
    let output = cmd
        .forge_fuse()
        .args(["test", "--strict-assembly"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("[FAIL: EvmError: Revert] test_First()"));
    assert!(output.contains("[FAIL: EvmError: Revert] test_Second()"));
});
