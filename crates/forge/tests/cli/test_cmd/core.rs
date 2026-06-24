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
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

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
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

// `forge --machine test` on passing tests: NDJSON stream + success envelope.
// Asserts record shape, per-suite ordering, and envelope payload.
forgetest_init!(machine_mode_emits_ndjson_stream, |prj, cmd| {
    use std::collections::HashSet;
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

    // Per-suite lifecycle: every opened suite is closed exactly once, and no
    // record of any kind targets a suite after its `suite_finished`.
    let mut saw_test_result = false;
    let mut opened_suites: HashSet<String> = HashSet::new();
    let mut closed_suites: HashSet<String> = HashSet::new();
    for line in &lines[..lines.len() - 1] {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-json stream line: {line}: {e}"));
        assert_eq!(v["schema_id"], "foundry:forge.test.event@v1");
        assert_eq!(v["command_id"], "forge.test");
        let ts = v["ts"].as_str().unwrap_or_else(|| panic!("missing ts on line: {line}"));
        chrono::DateTime::parse_from_rfc3339(ts)
            .unwrap_or_else(|e| panic!("ts `{ts}` not RFC 3339 on line {line}: {e}"));
        let suite = v["suite"].as_str().unwrap_or_else(|| panic!("missing suite: {line}"));
        let kind = v["kind"].as_str().unwrap_or("");
        assert!(
            !closed_suites.contains(suite),
            "`{kind}` for `{suite}` after its suite_finished: {line}"
        );
        opened_suites.insert(suite.to_string());
        match kind {
            "test_result" => saw_test_result = true,
            "suite_finished" => {
                assert!(
                    closed_suites.insert(suite.to_string()),
                    "duplicate suite_finished for `{suite}`: {line}"
                );
            }
            "warning" => {}
            other => panic!("unexpected event kind `{other}` on line: {line}"),
        }
    }
    assert!(saw_test_result, "missing any test_result event in: {stdout}");
    assert_eq!(
        opened_suites, closed_suites,
        "every opened suite must be terminated by a suite_finished"
    );

    // Terminal envelope.
    let envelope: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["success"], true);
    assert!(envelope["data"]["passed"].as_u64().is_some(), "missing passed: {envelope}");
    assert!(envelope["data"]["failed"].as_u64().is_some(), "missing failed: {envelope}");
    assert!(envelope["data"]["suites"].as_u64().is_some(), "missing suites: {envelope}");
});

// Failing test → error envelope + exit `TestFailure (5)`.
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
});

// Rejection envelope: stable `code`, exit code, structured `details.unsupported_flags`.
forgetest_init!(machine_mode_rejects_unsupported_flags, |_prj, cmd| {
    let assert = cmd.args(["--machine", "test", "--gas-report"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--gas-report"), "missing --gas-report mention: {envelope}");
    assert_eq!(
        envelope["errors"][0]["details"]["unsupported_flags"],
        serde_json::json!(["--gas-report"]),
        "missing structured unsupported_flags details: {envelope}"
    );
});

forgetest_init!(machine_mode_rejects_mutation_testing, |_prj, cmd| {
    let assert = cmd.args(["--machine", "test", "--mutate"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--mutate"), "missing --mutate mention: {envelope}");
    assert_eq!(
        envelope["errors"][0]["details"]["unsupported_flags"],
        serde_json::json!(["--mutate"]),
        "missing structured unsupported_flags details: {envelope}"
    );
});

// `--allow-failure`: success envelope with `data.failed > 0` and exit 0.
forgetest_init!(machine_mode_allow_failure_emits_success_envelope, |prj, cmd| {
    prj.add_test(
        "MachineAllowFailure.t.sol",
        r#"
import "forge-std/Test.sol";
contract MachineAllowFailureTest is Test {
    function testAlwaysFails() public { assertTrue(false); }
}
"#,
    );
    let assert = cmd.args(["--machine", "test", "--allow-failure"]).assert_success();
    assert_eq!(assert.get_output().status.code(), Some(0));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    let envelope: Value = serde_json::from_str(lines.last().unwrap())
        .unwrap_or_else(|e| panic!("trailing line not envelope: {stdout}: {e}"));
    assert_eq!(envelope["success"], true);
    assert_eq!(envelope["errors"], serde_json::json!([]));
    let failed = envelope["data"]["failed"].as_u64().unwrap_or(0);
    assert!(failed >= 1, "expected at least one tolerated failure in data: {envelope}");
});

// Main-compile errors → `compiler.solc.error` + `Build (4)`, not `cli.unknown`.
forgetest_init!(machine_mode_compile_failure_emits_typed_envelope, |prj, cmd| {
    prj.add_test(
        "BadCompile.t.sol",
        r#"
import "forge-std/Test.sol";
contract BadCompileTest is Test {
    function testWillNotCompile() public { this is not valid solidity; }
}
"#,
    );
    let assert = cmd.args(["--machine", "test"]).assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(4));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "compiler.solc.error");
});

// Same contract with a filter so the precompile path runs; same typed envelope.
forgetest_init!(machine_mode_precompile_failure_emits_typed_envelope, |prj, cmd| {
    prj.add_test(
        "BadCompilePrecompile.t.sol",
        r#"
import "forge-std/Test.sol";
contract BadCompilePrecompileTest is Test {
    function testWillNotCompile() public { this is not valid solidity; }
}
"#,
    );
    let assert =
        cmd.args(["--machine", "test", "--match-test", "testWillNotCompile"]).assert_failure();
    assert_eq!(assert.get_output().status.code(), Some(4));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "compiler.solc.error");
});

// `live_logs = true` in foundry.toml is silently overridden under `--machine`.
forgetest_init!(machine_mode_overrides_live_logs_config, |prj, cmd| {
    prj.update_config(|c| c.live_logs = true);
    prj.add_test(
        "LiveLogs.t.sol",
        r#"
import "forge-std/Test.sol";
import "forge-std/console.sol";
contract LiveLogsTest is Test {
    function testLogs() public { console.log("HUMAN_PROSE_LINE"); assertTrue(true); }
}
"#,
    );
    let assert = cmd.args(["--machine", "test"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("HUMAN_PROSE_LINE"),
        "raw console.log leaked to stdout under --machine: {stdout}"
    );
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("non-json stdout line `{line}`: {e}"));
    }
});

// `show_progress = true` in foundry.toml is silently overridden under `--machine`.
forgetest_init!(machine_mode_overrides_show_progress_config, |prj, cmd| {
    prj.update_config(|c| c.show_progress = true);
    prj.add_test(
        "Progress.t.sol",
        r#"
import "forge-std/Test.sol";
contract ProgressTest is Test {
    function testOne() public { assertTrue(true); }
    function testTwo() public { assertTrue(true); }
}
"#,
    );
    let assert = cmd.args(["--machine", "test"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("non-json stdout line `{line}`: {e}"));
    }
});

// `--watch` is dispatched before `cmd.run()`; guards the top-level preflight.
forgetest_init!(machine_mode_rejects_watch, |_prj, cmd| {
    let assert = cmd.args(["--machine", "test", "--watch"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
    let msg = envelope["errors"][0]["message"].as_str().unwrap_or("");
    assert!(msg.contains("--watch"), "missing --watch mention: {envelope}");
});

// Unadopted forge subcommands (snapshot, coverage, ...) must reject `--machine`
// at the top level — otherwise they'd inherit TestArgs's stream emission and
// spoof `command_id: forge.test` without ever emitting a terminal envelope.
forgetest_init!(machine_mode_rejects_unadopted_subcommand, |_prj, cmd| {
    let assert = cmd.args(["--machine", "snapshot"]).assert_failure();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let envelope: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("expected single-envelope error stdout: {stdout}: {e}"));
    assert_eq!(envelope["success"], false);
    assert_eq!(envelope["errors"][0]["code"], "cli.usage.invalid");
    assert_eq!(assert.get_output().status.code(), Some(2));
    assert_eq!(envelope["errors"][0]["details"]["subcommand"], "snapshot");
    // No spurious stream events leaked before the rejection envelope.
    assert!(
        !stdout.contains("forge.test.event"),
        "unadopted subcommand emitted stream events: {stdout}"
    );
});

// Warning duality: same `code`/`message`/`suite` on stream + envelope surfaces.
forgetest_init!(machine_mode_warning_appears_in_stream_and_envelope, |prj, cmd| {
    // Mis-cased `setup()` triggers a SuiteResult warning.
    prj.add_test(
        "MachineWarning.t.sol",
        r#"
import "forge-std/Test.sol";
contract MachineWarningTest is Test {
    function setup() public {}
    function testPasses() public { assertTrue(true); }
}
"#,
    );
    let assert = cmd.args(["--machine", "test"]).assert_success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();

    let mut stream_warning: Option<Value> = None;
    for line in &lines[..lines.len() - 1] {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["kind"] == "warning" {
            stream_warning = Some(v);
            break;
        }
    }
    let stream_warning = stream_warning.expect("no stream warning event emitted");
    assert_eq!(stream_warning["code"], "test.warning");
    let stream_message = stream_warning["message"].as_str().unwrap();
    let stream_suite = stream_warning["suite"].as_str().unwrap();

    let envelope: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    let envelope_warning = envelope["warnings"]
        .as_array()
        .and_then(|ws| ws.iter().find(|w| w["code"] == "test.warning"))
        .expect("envelope warnings[] missing test.warning entry");
    assert_eq!(envelope_warning["message"].as_str().unwrap(), stream_message);
    assert_eq!(envelope_warning["details"]["suite"].as_str().unwrap(), stream_suite);
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
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

forgetest_init!(rerun_filters_same_named_tests_by_contract, |prj, cmd| {
    prj.add_test(
        "RerunSameName.t.sol",
        r#"
import "forge-std/Test.sol";

contract FailingSameNameTest is Test {
    function testSharedName() public {
        assertTrue(false);
    }
}

contract PassingSameNameTest is Test {
    function testSharedName() public {
        assertTrue(true);
    }
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/RerunSameName.t.sol:FailingSameNameTest
[FAIL: assertion failed] testSharedName() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/RerunSameName.t.sol:PassingSameNameTest
[PASS] testSharedName() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)
...
"#]]);

    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/RerunSameName.t.sol:FailingSameNameTest
[FAIL: assertion failed] testSharedName() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 1 test suite [ELAPSED]: 0 tests passed, 1 failed, 0 skipped (1 total tests)
...
"#]]);
});

forgetest_init!(rerun_with_only_setup_failure_runs_all_tests, |prj, cmd| {
    prj.add_test(
        "RerunSetupFail.t.sol",
        r#"
import "forge-std/Test.sol";

contract OnlySetupFails is Test {
    function setUp() public {
        assertTrue(false);
    }

    function testA() public {
        assertTrue(true);
    }
}

contract HealthyContract is Test {
    function testC() public {
        assertTrue(true);
    }
}
"#,
    );

    cmd.args(["test", "-j1"]).assert_failure();

    // With no replayable failures recorded, `--rerun` falls back to a regular run instead of
    // selecting zero tests.
    cmd.forge_fuse().args(["test", "--rerun", "-j1"]).assert_failure().stdout_eq(str![[r#"
No files changed, compilation skipped

Ran 1 test for test/RerunSetupFail.t.sol:HealthyContract
[PASS] testC() ([GAS])
Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]

Ran 1 test for test/RerunSetupFail.t.sol:OnlySetupFails
[FAIL: assertion failed] setUp() ([GAS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]

Ran 2 test suites [ELAPSED]: 1 tests passed, 1 failed, 0 skipped (2 total tests)
...
"#]]);
});
