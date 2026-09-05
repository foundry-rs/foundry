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

function setUp() { sstore(0, 42) }

function test_min() {
    if iszero(eq(2, min(4, 2))) { revert(0, 0) }
}

function test_set_up_state() {
    if iszero(eq(sload(0), 42)) { revert(0, 0) }
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
    test_set_up_state


"#]]);

    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_success().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 2 tests for test/MathUtil.t.yul:MathUtil
[PASS] test_min() ([GAS])
[PASS] test_set_up_state() ([GAS])
Suite result: ok. 2 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 2 tests passed, 0 failed, 0 skipped (2 total tests)

"#]]);
});

forgetest!(strict_assembly_preserves_independent_yul_objects, |prj, cmd| {
    prj.create_file("test/Suite.t.yul", "function test_ok() {}\n");
    for object in ["First", "Second"] {
        prj.create_file(
            format!("src/{object}.yul"),
            &format!(
                r#"object "{object}" {{
    code {{ mstore(0, 1) return(0, 32) }}
}}"#
            ),
        );
    }

    cmd.args(["test", "--strict-assembly", "--list"]).assert_success();
    for object in ["First", "Second"] {
        assert!(prj.artifacts().join(format!("{object}.yul/{object}.json")).exists());
    }

    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_success();
    for object in ["First", "Second"] {
        prj.create_file(
            format!("src/{object}.yul"),
            &format!(
                r#"object "{object}" {{
    code {{ mstore(0, 2) return(0, 32) }}
}}"#
            ),
        );
    }
    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_success();
});

forgetest!(strict_assembly_invalidates_solidity_body_changes, |prj, cmd| {
    prj.create_file(
        "src/Value.sol",
        "contract Value { function value() external pure returns (uint256) { return 1; } }",
    );
    prj.create_file(
        "test/Value.t.sol",
        r#"
import {Value} from "../src/Value.sol";
contract ValueTest {
    function test_value() external { require(new Value().value() == 1); }
}
"#,
    );
    prj.create_file("test/Suite.t.yul", "function test_ok() {}\n");

    cmd.args(["test", "--strict-assembly"]).assert_success();
    prj.create_file(
        "src/Value.sol",
        "contract Value { function value() external pure returns (uint256) { return 2; } }",
    );
    let output = cmd
        .forge_fuse()
        .args(["test", "--strict-assembly"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(output.contains("ValueTest"));
    assert!(output.contains("[FAIL") && output.contains("test_value()"));

    prj.update_config(|config| config.dynamic_test_linking = true);
    prj.create_file(
        "src/Value.sol",
        "contract Value { function value() external pure returns (uint256) { return 1; } }",
    );
    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_success();
    prj.create_file(
        "src/Value.sol",
        "contract Value { function value() external pure returns (uint256) { return 2; } }",
    );
    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_failure();
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

forgetest!(strict_assembly_cache_is_not_reused_without_flag, |prj, cmd| {
    prj.create_file("test/Cache.t.yul", "function test_cache_identity() {}\n");

    cmd.args(["test", "--strict-assembly"]).assert_success();
    cmd.forge_fuse().arg("test").assert_failure();

    prj.update_config(|config| config.dynamic_test_linking = true);
    cmd.forge_fuse().args(["test", "--strict-assembly"]).assert_success();
    cmd.forge_fuse().arg("test").assert_failure();
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
