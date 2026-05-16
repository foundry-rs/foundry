//! Core test functionality tests

use foundry_test_utils::str;
use serde_json::Value;

forgetest_init!(failing_test_after_failed_setup, |prj, cmd| {
    prj.add_test(
        "FailingTestAfterFailedSetup.t.sol",
        r#"
import "forge-std/Test.sol";

contract FailingTestAfterFailedSetupTest is Test {
    function setUp() public {
        assertTrue(false);
    }

    function testAssertSuccess() public {
        assertTrue(true);
    }

    function testAssertFailure() public {
        assertTrue(false);
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/FailingTestAfterFailedSetup.t.sol:FailingTestAfterFailedSetupTest
[FAIL: assertion failed] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/FailingTestAfterFailedSetup.t.sol:FailingTestAfterFailedSetupTest
[FAIL: assertion failed] setUp() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

forgetest_init!(legacy_assertions, |prj, cmd| {
    prj.add_test(
        "LegacyAssertions.t.sol",
        r#"
import "forge-std/Test.sol";

contract NoAssertionsRevertTest is Test {
    function testMultipleAssertFailures() public {
        vm.assertEq(uint256(1), uint256(2));
        vm.assertLt(uint256(5), uint256(4));
    }
}

/// forge-config: default.legacy_assertions = true
contract LegacyAssertionsTest {
    bool public failed;

    function testFlagNotSetSuccess() public {}

    function testFlagSetFailure() public {
        failed = true;
    }
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Ran 2 tests for test/LegacyAssertions.t.sol:LegacyAssertionsTest
[PASS] testFlagNotSetSuccess() ([GAS])
[FAIL] testFlagSetFailure() ([GAS])
Suite result: FAILED. 1 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/LegacyAssertions.t.sol:NoAssertionsRevertTest
[FAIL: assertion failed: 1 != 2] testMultipleAssertFailures() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 2 failed, 0 skipped (3 total tests)

Failing tests:
Encountered 1 failing test in test/LegacyAssertions.t.sol:LegacyAssertionsTest
[FAIL] testFlagSetFailure() ([GAS])

Encountered 1 failing test in test/LegacyAssertions.t.sol:NoAssertionsRevertTest
[FAIL: assertion failed: 1 != 2] testMultipleAssertFailures() ([GAS])

Encountered a total of 2 failing tests, 1 tests succeeded

Tip: Run `forge test --rerun` to retry only the 2 failed tests

"#]]);
});

// `forge --machine test` emits NDJSON: per-test events, suite_finished
// events, terminating in a success envelope. Exercises the canonical
// passing-tests path with a small fixture suite.
forgetest_init!(machine_mode_emits_ndjson_stream, |prj, cmd| {
    prj.add_test(
        "MachinePass.t.sol",
        r#"
import "forge-std/Test.sol";
contract MachinePassTest is Test {
    function testAlwaysPasses() public { assertTrue(true); }
    function testAlsoPasses() public { assertEq(uint256(1), uint256(1)); }
}
"#,
    );
    let assert = cmd.args(["--machine", "test"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    assert!(lines.len() >= 2, "expected stream + envelope lines, got: {stdout}");

    // Every record parses as JSON and stream events carry the spec fields.
    let mut saw_test_result = false;
    let mut saw_suite_finished = false;
    for line in &lines[..lines.len() - 1] {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-json stream line: {line}: {e}"));
        foundry_test_utils::agent_schema::validate("foundry:forge.test.event@v1", &v);
        match v["kind"].as_str().unwrap_or("") {
            "test_result" => saw_test_result = true,
            "suite_finished" => saw_suite_finished = true,
            other => panic!("unexpected event kind `{other}` on line: {line}"),
        }
    }
    assert!(saw_test_result, "missing any test_result event in: {stdout}");
    assert!(saw_suite_finished, "missing any suite_finished event in: {stdout}");

    // Terminal envelope: 2-test, 1-suite fixture pinned exactly.
    let envelope: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["data"]["passed"].as_u64(), Some(2));
    assert_eq!(envelope["data"]["failed"].as_u64(), Some(0));
    assert_eq!(envelope["data"]["suites"].as_u64(), Some(1));
    foundry_test_utils::agent_schema::validate_envelope_data(&envelope, "foundry:forge.test@v1");

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// On a failing test, `forge --machine test` ends with an error envelope and
// exits with `ExitCode::TestFailure` (5).
forgetest_init!(machine_mode_failing_test_emits_error_envelope, |prj, cmd| {
    prj.add_test(
        "MachineFail.t.sol",
        r#"
import "forge-std/Test.sol";
contract MachineFailTest is Test {
    function testAlwaysFails() public { assertTrue(false); }
}
"#,
    );
    let assert = cmd.args(["--machine", "test"]).assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(5));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    let envelope: Value = serde_json::from_str(lines.last().unwrap())
        .unwrap_or_else(|e| panic!("trailing line not envelope: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "test.failed");
    let failed = envelope["errors"][0]["details"]["failed"].as_u64().unwrap_or(0);
    assert!(failed >= 1, "expected at least one failed test in details: {envelope}");
    foundry_test_utils::agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `--machine` rejects flags that would corrupt the NDJSON stream contract.
forgetest_init!(machine_mode_rejects_unsupported_flags, |_prj, cmd| {
    let assert = cmd.args(["--machine", "test", "--gas-report"]).assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(2));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert!(envelope["data"].is_null(), "data must be null on failure: {envelope}");
    assert_eq!(envelope["warnings"], serde_json::json!([]));
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--gas-report"), "missing --gas-report mention: {envelope}");
    foundry_test_utils::agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

// `--watch` is dispatched separately from `cmd.run()`, so the rejection
// preflight has to live at the top-level entry. This regression guards
// against the dispatch boundary moving back under the rejection path.
forgetest_init!(machine_mode_rejects_watch, |_prj, cmd| {
    let assert = cmd.args(["--machine", "test", "--watch"]).assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(2));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert!(envelope["data"].is_null(), "data must be null on failure: {envelope}");
    assert_eq!(envelope["warnings"], serde_json::json!([]));
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--watch"), "missing --watch mention: {envelope}");
    foundry_test_utils::agent_schema::validate("foundry:envelope@v1", &envelope);

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.is_empty(), "stderr must be empty under --machine, got: {stderr}");
});

forgetest_init!(payment_failure, |prj, cmd| {
    prj.add_test(
        "PaymentFailure.t.sol",
        r#"
import "forge-std/Test.sol";

contract Payable {
    function pay() public payable {}
}

contract PaymentFailureTest is Test {
    function testCantPay() public {
        Payable target = new Payable();
        vm.prank(address(1));
        target.pay{value: 1}();
    }
}
"#,
    );

    cmd.arg("test").assert_failure().stdout_eq(str![[r#"
[COMPILING_FILES] with [SOLC_VERSION]
[SOLC_VERSION] [ELAPSED]
Compiler run successful!

Ran 1 test for test/PaymentFailure.t.sol:PaymentFailureTest
[FAIL: EvmError: Revert] testCantPay() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)

Failing tests:
Encountered 1 failing test in test/PaymentFailure.t.sol:PaymentFailureTest
[FAIL: EvmError: Revert] testCantPay() ([GAS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
