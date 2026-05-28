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
// passing-tests path with a small fixture suite and asserts the contract
// shape (schema_id / command_id / RFC 3339 ts), per-suite ordering
// (`test_result*` then `suite_finished` then no more `test_result` for
// that suite), and the terminal envelope payload.
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

    // Every record parses as JSON and stream events carry the spec fields.
    // Per-suite ordering: `test_result`s for a suite all precede that
    // suite's `suite_finished`. Once `suite_finished` for a contract has
    // fired, no further `test_result` may target that contract.
    let mut saw_test_result = false;
    let mut saw_suite_finished = false;
    let mut closed_suites: HashSet<String> = HashSet::new();
    for line in &lines[..lines.len() - 1] {
        let v: Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("non-json stream line: {line}: {e}"));
        assert_eq!(v["schema_id"], "foundry:forge.test.event@v1");
        assert_eq!(v["command_id"], "forge.test");
        // `ts` must round-trip through RFC 3339 — agents pin this contract.
        let ts = v["ts"].as_str().unwrap_or_else(|| panic!("missing ts on line: {line}"));
        chrono::DateTime::parse_from_rfc3339(ts)
            .unwrap_or_else(|e| panic!("ts `{ts}` not RFC 3339 on line {line}: {e}"));
        let contract = v["contract"].as_str().unwrap_or_else(|| panic!("missing contract: {line}"));
        match v["kind"].as_str().unwrap_or("") {
            "test_result" => {
                assert!(
                    !closed_suites.contains(contract),
                    "test_result for `{contract}` after its suite_finished: {line}"
                );
                saw_test_result = true;
            }
            "suite_finished" => {
                assert!(
                    closed_suites.insert(contract.to_string()),
                    "duplicate suite_finished for `{contract}`: {line}"
                );
                saw_suite_finished = true;
            }
            // `warning` is allowed inter-test under the spec; nothing to assert here.
            "warning" => {}
            other => panic!("unexpected event kind `{other}` on line: {line}"),
        }
    }
    assert!(saw_test_result, "missing any test_result event in: {stdout}");
    assert!(saw_suite_finished, "missing any suite_finished event in: {stdout}");

    // Terminal envelope.
    let envelope: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(envelope["schema_version"], 1);
    assert_eq!(envelope["success"], true);
    assert!(envelope["data"]["passed"].as_u64().is_some(), "missing passed: {envelope}");
    assert!(envelope["data"]["failed"].as_u64().is_some(), "missing failed: {envelope}");
    assert!(envelope["data"]["suites"].as_u64().is_some(), "missing suites: {envelope}");
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
});

// `--machine` rejects flags that would corrupt the NDJSON stream contract.
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
});

// Compile errors at the main compile site (empty filter → precompile is
// skipped) must surface as `compiler.solc.error` + `Build (4)` under
// `--machine`, not as the generic `cli.unknown` + exit `1` that an
// untyped `eyre::bail!("Compilation failed")` would produce.
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

// Same contract but with a filter set so `get_sources_to_compile` runs the
// precompile path before the main compile. Both sites must emit the same
// typed envelope, not the generic `cli.unknown` path.
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

// `live_logs = true` in foundry.toml would normally print console.log output
// straight to stdout, corrupting the NDJSON stream. Under `--machine` the
// config knob must be silently neutralized (the CLI equivalent `--live-logs`
// is rejected separately in `machine_mode_rejects_unsupported_flags`).
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
    // Every stdout line is valid JSON; no human prose survived.
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("non-json stdout line `{line}`: {e}"));
    }
});

// `show_progress = true` in foundry.toml would enable progress mode in
// `multi_runner`, which batches results and prints progress UI — neither
// compatible with real-time NDJSON streaming. Override must be silent.
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
    // Every stdout line must be valid JSON; progress bars and human text would not be.
    for line in stdout.lines().filter(|l| !l.is_empty()) {
        serde_json::from_str::<Value>(line)
            .unwrap_or_else(|e| panic!("non-json stdout line `{line}`: {e}"));
    }
});

// `--watch` is dispatched separately from `cmd.run()`, so the rejection
// preflight has to live at the top-level entry. This regression guards
// against the dispatch boundary moving back under the rejection path.
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
