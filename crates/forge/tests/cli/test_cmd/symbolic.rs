use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, str, util::OutputExt};
use serde_json::Value;
use std::{path::PathBuf, process::Command};

use super::symbolic_helpers::{assert_relevant_lines, assert_symbolic};

fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

fn json_test_result(stdout: &[u8], signature: &str) -> Value {
    let json: Value = serde_json::from_slice(stdout).expect("forge test --json output");
    let suites = json.as_object().expect("top-level suites object");
    for suite in suites.values() {
        if let Some(result) = suite["test_results"].get(signature) {
            return result.clone();
        }
    }
    panic!("missing JSON test result for {signature}: {json}");
}

fn read_artifact_ref(artifact_ref: &Value) -> Value {
    let artifact_path = artifact_ref["path"].as_str().expect("symbolic artifact path");
    let artifact_path = PathBuf::from(artifact_path);
    let artifact = std::fs::read_to_string(&artifact_path)
        .unwrap_or_else(|err| panic!("failed to read artifact {}: {err}", artifact_path.display()));
    serde_json::from_str(&artifact).expect("symbolic counterexample artifact")
}

fn read_artifact(symbolic: &Value) -> Value {
    read_artifact_ref(&symbolic["artifact"])
}

forgetest_init!(symbolic_tests_are_ignored_without_flag, |prj, cmd| {
    prj.add_test(
        "SymbolicIgnored.t.sol",
        r#"
contract SymbolicIgnored {
    function checkWouldFail(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--match-test", "checkWouldFail"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
No tests found
"#]],
    );
});

forgetest_init!(symbolic_passes_scalar_test, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!("skipping symbolic_passes_scalar_test because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicPass.t.sol",
        r#"
contract SymbolicPass {
    function checkNoop(uint256) public pure {}
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkNoop"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkNoop(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
(paths:
"#]],
    );
});

forgetest_init!(symbolic_json_schema_reports_pass, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_json_schema_reports_pass because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicJsonPass.t.sol",
        r#"
contract SymbolicJsonPass {
    function checkNoop(uint256) public pure {}
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkNoop"])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkNoop(uint256)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["schema_version"], 1);
    assert_eq!(symbolic["status"], "pass");
    assert!(symbolic["incomplete"].is_null());
    assert_eq!(symbolic["replay"]["required"], false);
    assert_eq!(symbolic["replay"]["status"], "not_required");
    assert!(symbolic["counterexample"].is_null());
    assert_eq!(symbolic["bounds"]["max_paths"], 1024);
    assert_eq!(symbolic["solver"]["name"], "z3");
    assert!(symbolic["solver"]["stats"]["paths"].as_u64().unwrap() >= 1);
    assert_eq!(symbolic["assumptions"][0]["kind"], "bounded_exploration");
});

forgetest_init!(symbolic_loop_bound_limits_symbolic_unrolling, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_loop_bound_limits_symbolic_unrolling because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicLoopBound.t.sol",
        r#"
contract SymbolicLoopBound {
    /// forge-config: default.symbolic.loop = 2
    function checkLoopBound(uint8 n) public pure {
        uint256 i;
        while (i < n) {
            ++i;
        }
        assert(i <= 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkLoopBound"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkLoopBound(uint8)
"#]],
    );
    assert!(!stdout.contains("symbolic depth limit exceeded"), "{stdout}");
});

forgetest_init!(symbolic_finds_assert_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_finds_assert_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicAssert.t.sol",
        r#"
contract SymbolicAssert {
    function checkRejectsFortyTwo(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--match-test", "checkRejectsFortyTwo"])
        .assert_failure()
        .get_output()
        .clone();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
panic: assertion failed
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkRejectsFortyTwo(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=[42]
"#]],
    );
    assert!(stderr.contains("Counterexample artifact:"), "{stderr}");
    assert!(stderr.contains("cache/symbolic/"), "{stderr}");
});

forgetest_init!(symbolic_json_schema_reports_replayed_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_json_schema_reports_replayed_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicJsonCounterexample.t.sol",
        r#"
contract SymbolicJsonCounterexample {
    function checkRejectsFortyTwo(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );
    prj.add_test(
        "SymbolicJsonCounterexampleDuplicate.t.sol",
        r#"
contract SymbolicJsonCounterexample {
    function checkRejectsFortyTwo(uint256) public pure {}
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--json",
            "--match-test",
            "checkRejectsFortyTwo",
            "--match-path",
            "test/SymbolicJsonCounterexample.t.sol",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkRejectsFortyTwo(uint256)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert!(symbolic["incomplete"].is_null());
    assert_eq!(symbolic["replay"]["required"], true);
    assert_eq!(symbolic["replay"]["status"], "confirmed");
    assert!(symbolic["counterexample"]["calldata"].as_str().unwrap().starts_with("0x"));
    assert_eq!(symbolic["counterexample"]["args"], "42");
    assert_eq!(symbolic["counterexample"]["raw_args"], "42");
    assert_eq!(symbolic["artifact"]["schema"], "foundry:symbolic.counterexample@v1");
    assert_eq!(result["counterexample_artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(result["counterexample_artifacts"][0], symbolic["artifact"]);
    let artifact_path = symbolic["artifact"]["path"].as_str().unwrap().to_string();
    let artifact = read_artifact(symbolic);
    assert_eq!(artifact["schema_version"], 1);
    assert_eq!(artifact["schema"], "foundry:symbolic.counterexample@v1");
    assert_eq!(artifact["kind"], "single_call");
    assert_eq!(artifact["test"]["test"], "checkRejectsFortyTwo(uint256)");
    assert_eq!(artifact["replay"]["status"], "confirmed");
    assert_eq!(artifact["calls"].as_array().unwrap().len(), 1);
    assert!(artifact["calls"][0]["calldata"].as_str().unwrap().starts_with("0x"));
    assert_eq!(artifact["calls"][0]["args"], "42");
    assert_eq!(artifact["calls"][0]["raw_args"], "42");
    assert!(symbolic["solver"]["stats"]["model_queries"].as_u64().unwrap() >= 1);

    let replay_stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", &artifact_path])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
[FAIL;
"#]],
    );
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
args=[42]
"#]],
    );
    assert!(
        !replay_stdout.contains("SymbolicJsonCounterexampleDuplicate.t.sol"),
        "{replay_stdout}"
    );

    let mut invalid_artifact = artifact.clone();
    invalid_artifact["calls"][0]["value"] = serde_json::json!("0x1");
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&invalid_artifact).unwrap()).unwrap();
    let invalid_stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", &artifact_path])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(
        invalid_stdout
            .contains("single-call symbolic artifact replay does not support non-zero value"),
        "{invalid_stdout}"
    );
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    prj.add_test(
        "SymbolicJsonCounterexample.t.sol",
        r#"
contract SymbolicJsonCounterexample {
    function checkRejectsFortyTwo(uint256) public pure {}
}
"#,
    );
    cmd.forge_fuse().args(["test", "--replay-symbolic-artifact", &artifact_path]).assert_success();

    prj.add_test(
        "SymbolicJsonCounterexample.t.sol",
        r#"
contract SymbolicJsonCounterexample {
    function checkRenamed(uint256) public pure {}
}
"#,
    );
    let stale_stderr = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", &artifact_path])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(stale_stderr.contains("symbolic artifact target"), "{stale_stderr}");
    assert!(
        stale_stderr.contains("checkRejectsFortyTwo(uint256)` was not found"),
        "{stale_stderr}"
    );
});

forgetest_init!(symbolic_json_schema_reports_replay_skip, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_json_schema_reports_replay_skip because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicJsonReplaySkip.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicJsonReplaySkip is Test {
    function checkReplaySkip(uint256 x) public {
        uint256 startedAt = vm.unixTime();
        vm.sleep(500);
        vm.skip(vm.unixTime() >= startedAt + 250, "replay slept");
        assert(x != 42);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkReplaySkip"])
        .assert_success()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkReplaySkip(uint256)");
    assert_eq!(result["status"], "Skipped");
    assert_eq!(result["reason"], "replay slept");
    assert!(result["counterexample"].is_null());

    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "incomplete");
    assert_eq!(symbolic["incomplete"]["kind"], "error");
    assert_eq!(symbolic["replay"]["required"], true);
    assert_eq!(symbolic["replay"]["status"], "skipped");
    assert!(
        symbolic["replay"]["reason"].as_str().unwrap().contains("vm.skip during concrete replay")
    );
    assert_eq!(symbolic["counterexample"]["args"], "42");
    assert!(symbolic["artifact"].is_null());
});

forgetest_init!(symbolic_json_schema_reports_incomplete, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_json_schema_reports_incomplete because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicJsonIncomplete.t.sol",
        r#"
contract SymbolicJsonIncomplete {
    function checkWidth(uint8 x) public pure {
        uint256 acc;
        if ((x & 0x01) != 0) acc += 1; else acc += 2;
        if ((x & 0x02) != 0) acc += 4; else acc += 8;
        assert(acc != 0);
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--json",
            "--symbolic-width",
            "1",
            "--match-test",
            "checkWidth",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkWidth(uint8)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "incomplete");
    assert_eq!(symbolic["incomplete"]["kind"], "stuck");
    assert!(symbolic["incomplete"]["reason"].as_str().unwrap().contains("path limit"));
    assert_eq!(symbolic["bounds"]["max_paths"], 1);
    assert_eq!(symbolic["replay"]["status"], "not_required");
    assert!(symbolic["counterexample"].is_null());
});

forgetest_init!(symbolic_finds_wrapping_arithmetic_riddle_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_finds_wrapping_arithmetic_riddle_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRiddle.t.sol",
        r#"
contract SymbolicRiddle {
    function check_riddle(uint256 x) external pure {
        uint256 msgSender = uint160(0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38);

        unchecked {
            require(x * x < msgSender);
        }

        require(x > msgSender);
        require(x & 0x800 != 0);
        require(x & 0x10000 == 0);

        assert(false);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "check_riddle"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
panic: assertion failed
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
check_riddle(uint256)
"#]],
    );
    assert!(!stdout.contains("unsupported symbolic execution feature"), "{stdout}");
});

forgetest_init!(symbolic_ignores_plain_require_revert, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_ignores_plain_require_revert because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRequire.t.sol",
        r#"
contract SymbolicRequire {
    function checkRequire(uint256 x) public pure {
        require(x != 42, "hit");
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkRequire"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkRequire(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
(paths:
"#]],
    );
});

forgetest_init!(symbolic_vm_assume_prunes_paths, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_vm_assume_prunes_paths because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicAssume.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicAssume is Test {
    function checkAssume(uint256 x) public {
        vm.assume(x != 42);
        assert(x != 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkAssume"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkAssume(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
(paths:
"#]],
    );
});

forgetest_init!(symbolic_finds_bytes_counterexample_with_native_inline_config, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_finds_bytes_counterexample_with_native_inline_config because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicBytes.t.sol",
        r#"
contract SymbolicBytes {
    /// forge-config: default.symbolic.array_lengths = [3]
    function checkBytes(bytes memory data) public pure {
        if (data[1] == 0x42) {
            assert(false);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkBytes"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkBytes(bytes)
"#]],
    );
});

forgetest_init!(symbolic_replays_string_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_replays_string_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicString.t.sol",
        r#"
contract SymbolicString {
    /// forge-config: default.symbolic.array_lengths = [3]
    function checkString(string memory value) public pure {
        bytes memory data = bytes(value);
        if (data[0] == bytes1(uint8(0x41))) {
            assert(false);
        }
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkString"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkString(string)
"#]],
    );
});

forgetest_init!(symbolic_uses_native_array_lengths, |prj, cmd| {
    if !z3_available() {
        let _ =
            sh_eprintln!("skipping symbolic_uses_native_array_lengths because z3 is not available");
        return;
    }

    prj.add_test(
        "SymbolicNativeArrayLengths.t.sol",
        r#"
contract SymbolicNativeArrayLengths {
    /// forge-config: default.symbolic.array_lengths = [3]
    function checkArray(uint256[] memory values) public pure {
        assert(values.length == 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkArray"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkArray(uint256[])
"#]],
    );
});

forgetest_init!(symbolic_uses_legacy_halmos_array_lengths, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_uses_legacy_halmos_array_lengths because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicHalmosLengths.t.sol",
        r#"
contract SymbolicHalmosLengths {
    /// @custom:halmos --array-lengths 3
    function checkArray(uint256[] memory values) public pure {
        assert(values.length == 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkArray"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkArray(uint256[])
"#]],
    );
});

forgetest_init!(symbolic_handles_nested_struct_dynamic_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_handles_nested_struct_dynamic_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicNestedStruct.t.sol",
        r#"
contract SymbolicNestedStruct {
    struct Payload {
        uint256[] values;
        bytes note;
    }

    /// forge-config: default.symbolic.array_lengths = [2, 3]
    function checkStruct(Payload memory payload) public pure {
        assert(payload.values.length == 2);
        assert(payload.note.length == 3);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkStruct"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkStruct((uint256[],bytes))
"#]],
    );
});

forgetest_init!(symbolic_allows_shorter_variants_with_positional_inner_lengths, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_allows_shorter_variants_with_positional_inner_lengths because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMixedLengthSets.t.sol",
        r#"
contract SymbolicMixedLengthSets {
    /// forge-config: default.symbolic.default_array_lengths = [1, 2]
    /// forge-config: default.symbolic.array_lengths = [4, 4]
    function checkBatch(bytes[] memory items) public pure {
        assert(items.length == 1 || items.length == 2);
        for (uint256 i; i < items.length; i++) {
            assert(items[i].length == 4);
        }
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkBatch"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicMixedLengthSets.t.sol:SymbolicMixedLengthSets
[PASS] checkBatch(bytes[]) ([METRICS])
...
"#]]);
});

forgetest_init!(symbolic_reports_calldata_variant_width_exhaustion, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_reports_calldata_variant_width_exhaustion because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicVariantLimit.t.sol",
        r#"
contract SymbolicVariantLimit {
    /// forge-config: default.symbolic.width = 2
    /// forge-config: default.symbolic.default_array_lengths = [1, 2]
    /// forge-config: default.symbolic.default_bytes_lengths = [1, 2]
    function checkVariants(bytes[] memory items) public pure {
        items;
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkVariants"]))
        .failure()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicVariantLimit.t.sol:SymbolicVariantLimit
[FAIL: incomplete symbolic execution (Stuck): symbolic calldata variant limit exceeded (2)] checkVariants(bytes[]) ([METRICS])
...
"#]]);
});

forgetest_init!(symbolic_rejects_malformed_halmos_array_lengths, |prj, cmd| {
    prj.add_test(
        "SymbolicMalformedHalmos.t.sol",
        r#"
contract SymbolicMalformedHalmos {
    /// forge-config: default.symbolic.default_dynamic_length = 2
    /// @custom:halmos --array-lengths nope
    function checkBytes(bytes memory data) public pure {
        data;
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--match-test", "checkBytes"])
        .assert_failure()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();

    assert_relevant_lines(
        &stderr,
        foundry_test_utils::str![[r#"
invalid @custom:halmos annotation
"#]],
    );
    assert_relevant_lines(
        &stderr,
        foundry_test_utils::str![[r#"
invalid length `nope`
"#]],
    );
});

forgetest_init!(symbolic_selfdestruct_cancun_self_beneficiary_halts, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_selfdestruct_cancun_self_beneficiary_halts because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSelfdestructCancun.t.sol",
        r#"
import "forge-std/Test.sol";

/// forge-config: default.evm_version = "cancun"

contract SymbolicSelfdestructCancun is Test {
    function checkSelfdestructCancun(uint256) public {
        selfdestruct(payable(address(this)));

        assert(false);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkSelfdestructCancun"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkSelfdestructCancun(uint256)
"#]],
    );
    assert!(!stdout.contains("SELFDESTRUCT/EIP-6780 not modeled"), "{stdout}");
});
forgetest_init!(symbolic_invariant_finds_single_step_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_invariant_finds_single_step_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantSingle.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicCounterTarget {
    uint256 public value;

    function set(uint256 x) external {
        if (x == 7) {
            value = 11;
        }
    }
}

contract SymbolicInvariantSingle is Test {
    SymbolicCounterTarget target;

    function setUp() public {
        target = new SymbolicCounterTarget();
        targetContract(address(target));
    }

    function invariant_counterNeverEleven() public view {
        assert(target.value() != 11);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--match-test", "invariant_counterNeverEleven"])
        .assert_failure()
        .get_output()
        .clone();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
invariant_counterNeverEleven()
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
set(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=[7]
"#]],
    );
    assert!(stderr.contains("Counterexample artifact:"), "{stderr}");
    assert!(stderr.contains("cache/symbolic/"), "{stderr}");
    assert!(!stdout.contains("No contracts to fuzz"), "{stdout}");
});

forgetest_init!(symbolic_json_reports_sequence_counterexample_artifact, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_json_reports_sequence_counterexample_artifact because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantSequenceArtifact.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicArtifactTarget {
    uint256 public value;

    function set(uint256 x) external {
        if (x == 7) {
            value = 11;
        }
    }
}

contract SymbolicInvariantSequenceArtifact is Test {
    SymbolicArtifactTarget target;

    function setUp() public {
        target = new SymbolicArtifactTarget();
        targetContract(address(target));
    }

    function invariant_counterNeverEleven() public view {
        assert(target.value() != 11);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "invariant_counterNeverEleven"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "invariant_counterNeverEleven()");
    let failures = result["invariant_failures"].as_array().expect("invariant failures");
    let failure = failures.first().expect("invariant failure");
    assert_eq!(failure["artifact"]["schema"], "foundry:symbolic.counterexample@v1");
    assert_eq!(result["counterexample_artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(result["counterexample_artifacts"][0], failure["artifact"]);
    let artifact_path = failure["artifact"]["path"].as_str().unwrap().to_string();

    let artifact = read_artifact_ref(&failure["artifact"]);
    assert_eq!(artifact["schema_version"], 1);
    assert_eq!(artifact["schema"], "foundry:symbolic.counterexample@v1");
    assert_eq!(artifact["kind"], "sequence");
    assert_eq!(artifact["test"]["test"], "invariant_counterNeverEleven()");
    assert_eq!(artifact["replay"]["status"], "confirmed");
    assert_eq!(artifact["calls"].as_array().unwrap().len(), 1);
    assert_eq!(artifact["calls"][0]["function_name"], "set");
    assert_eq!(artifact["calls"][0]["args"], "7");

    let replay_stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", &artifact_path])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
invariant_counterNeverEleven()
"#]],
    );
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
args=[7]
"#]],
    );
});

forgetest_init!(symbolic_invariant_respects_sequence_depth, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_invariant_respects_sequence_depth because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantDepth.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicDepthTarget {
    uint256 public value;

    function arm(uint256 x) external {
        if (x == 1) {
            value = 1;
        }
    }

    function trip(uint256 x) external {
        if (value == 1 && x == 2) {
            value = 2;
        }
    }
}

contract SymbolicInvariantDepth is Test {
    SymbolicDepthTarget target;

    function setUp() public {
        target = new SymbolicDepthTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_valueBelowTwo() public view {
        assert(target.value() < 2);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "invariant_valueBelowTwo"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
arm(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
trip(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=[1]
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
args=[2]
"#]],
    );
});

forgetest_init!(symbolic_invariant_uses_target_sender, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_invariant_uses_target_sender because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantSender.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSenderTarget {
    address public lastSender;

    function touch(uint256 x) external {
        if (x == 3) {
            lastSender = msg.sender;
        }
    }
}

contract SymbolicInvariantSender is Test {
    SymbolicSenderTarget target;
    address constant BOB = address(0xB0B);

    function setUp() public {
        target = new SymbolicSenderTarget();
        targetContract(address(target));
        targetSender(BOB);
    }

    function invariant_senderIsNotBob() public view {
        assert(target.lastSender() != BOB);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "invariant_senderIsNotBob"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
touch(uint256)
"#]],
    );
    assert!(
        stdout.to_lowercase().contains("sender=0x0000000000000000000000000000000000000b0b"),
        "{stdout}"
    );
});
forgetest_init!(symbolic_soundness_hardening_regressions, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_soundness_hardening_regressions because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicSoundnessHardening.t.sol",
        r#"
import "forge-std/Test.sol";

interface SvmDynamic {
    function createUint(uint256 bits, string calldata name) external returns (uint256);
    function createInt(uint256 bits, string calldata name) external returns (int256);
}

contract DelegateTarget {
    function noop() external {}
}

contract SymbolicSoundnessHardening is Test {
    address constant SVM_ADDRESS = address(0xF3993A62377BCd56AE39D773740A5390411E8BC9);

    mapping(uint256 => uint256) values;
    DelegateTarget target;

    function setUp() public {
        target = new DelegateTarget();
    }

    function checkConstrainedStorageKeyUsesConcreteSlot(uint256 key) public {
        values[7] = 0xbeef;
        vm.assume(key == 7);
        assertEq(values[key], 0xbeef);
    }

    function checkRandomUintRejectsOversizedBits() public {
        vm.randomUint(257);
    }

    function checkCreateUintRejectsOversizedBits() public {
        SvmDynamic(SVM_ADDRESS).createUint(257, "too-wide");
    }

    function checkCreateIntRejectsOversizedBits() public {
        SvmDynamic(SVM_ADDRESS).createInt(300, "too-wide");
    }

    function checkPrankDelegatecallReportsUnsupported() public {
        vm.prank(address(0xB0B));
        (bool ok,) = address(target).delegatecall(abi.encodeWithSignature("noop()"));
        assertTrue(ok);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--match-contract", "SymbolicSoundnessHardening"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
[PASS] checkConstrainedStorageKeyUsesConcreteSlot(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic randomUint bits
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic svm.create integer bits
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic prank delegatecall
"#]],
    );
});
