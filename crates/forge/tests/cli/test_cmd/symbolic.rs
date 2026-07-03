use alloy_primitives::{U256, hex, keccak256};
use foundry_common::sh_eprintln;
use foundry_test_utils::{
    forgetest_init, str,
    util::{OutputExt, SOLC_VERSION},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

use super::symbolic_helpers::{
    assert_relevant_lines, assert_symbolic, json_test_result, read_artifact_ref,
};

fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

fn read_artifact(symbolic: &Value) -> Value {
    read_artifact_ref(&symbolic["artifact"])
}

fn frontier_artifact_path(root: &Path, contract: &str, test: &str) -> PathBuf {
    root.join("fuzz_frontiers").join(contract).join(test).join("branch-frontiers.json")
}

fn keep_only_matching_frontier(
    root: &Path,
    contract: &str,
    test: &str,
    missing: &str,
    mut matches: impl FnMut(&Value) -> bool,
) -> Value {
    let frontier_path = frontier_artifact_path(root, contract, test);
    let mut artifact: Value = serde_json::from_slice(
        &std::fs::read(&frontier_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", frontier_path.display())),
    )
    .unwrap();
    let target_frontier = artifact["frontiers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|frontier| matches(frontier))
        .cloned()
        .unwrap_or_else(|| panic!("missing {missing} frontier in {artifact}"));
    *artifact["frontiers"].as_array_mut().unwrap() = vec![target_frontier.clone()];
    std::fs::write(&frontier_path, serde_json::to_vec_pretty(&artifact).unwrap())
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", frontier_path.display()));
    target_frontier
}

fn matching_frontier(
    root: &Path,
    contract: &str,
    test: &str,
    missing: &str,
    mut matches: impl FnMut(&Value) -> bool,
) -> Value {
    let frontier_path = frontier_artifact_path(root, contract, test);
    let artifact: Value = serde_json::from_slice(
        &std::fs::read(&frontier_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", frontier_path.display())),
    )
    .unwrap();
    artifact["frontiers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|frontier| matches(frontier))
        .cloned()
        .unwrap_or_else(|| panic!("missing {missing} frontier in {artifact}"))
}

fn single_uint_corpus_values(
    root: &Path,
    contract: &str,
    test: &str,
    signature: &str,
) -> Vec<U256> {
    let corpus_dir =
        root.join("fuzz_corpus").join(contract).join(test).join("worker0").join("corpus");
    let expected_selector = format!("0x{}", hex::encode(&keccak256(signature.as_bytes())[..4]));
    let mut values = Vec::new();
    for entry in std::fs::read_dir(&corpus_dir)
        .unwrap_or_else(|err| panic!("failed to read corpus dir {}: {err}", corpus_dir.display()))
    {
        let entry = entry.unwrap();
        let corpus: Value = serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
        for tx in corpus.as_array().unwrap() {
            let calldata = tx["calldata"].as_str().expect("seed calldata");
            if calldata.starts_with(&expected_selector) {
                values.push(U256::from_be_slice(&hex::decode(&calldata[10..74]).unwrap()));
            }
        }
    }
    values
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

forgetest_init!(symbolic_contract_inline_config_enables_check_entrypoints, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_contract_inline_config_enables_check_entrypoints because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicContractInlineConfig.t.sol",
        r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

/// forge-config: default.symbolic.enabled = true
contract SymbolicContractInlineConfig {
    function checkEnabledByContractConfig(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--match-test", "checkEnabledByContractConfig"])
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
checkEnabledByContractConfig(uint256)
"#]],
    );
});

forgetest_init!(symbolic_single_call_artifact_replay_honors_env_fields, |prj, cmd| {
    prj.add_test(
        "SymbolicSingleCallArtifactEnv.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSingleCallArtifactEnv is Test {
    address constant BOB = address(0xB0B);

    function setUp() public {
        vm.warp(1000);
        vm.roll(2000);
        vm.deal(BOB, 2 ether);
    }

    function checkEnv() public payable {
        if (
            msg.sender == BOB
                && msg.value == 2 ether
                && block.timestamp == 1007
                && block.number == 2011
        ) {
            revert("artifact env replayed");
        }
    }
}
"#,
    );

    let artifact_path = prj.root().join("single-call-env-artifact.json");
    let selector = keccak256(b"checkEnv()");
    let artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "single_call",
        "test": {
            "contract": "test/SymbolicSingleCallArtifactEnv.t.sol:SymbolicSingleCallArtifactEnv",
            "test": "checkEnv()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": null
        },
        "replay_semantics": {
            "fail_on_revert": false
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 0,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [{
            "warp": "0x7",
            "roll": "0xb",
            "sender": "0x0000000000000000000000000000000000000b0b",
            "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
            "calldata": format!("0x{}", hex::encode(&selector[..4])),
            "value": format!("{:#x}", 3_000_000_000_000_000_000u128),
            "contract_name": "SymbolicSingleCallArtifactEnv",
            "function_name": "checkEnv",
            "signature": "checkEnv()",
            "args": "",
            "raw_args": ""
        }]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("artifact env replayed"), "{stdout}");
});

forgetest_init!(symbolic_emits_stateless_solidity_regression, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_stateless_solidity_regression because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRegressionSingle.t.sol",
        r#"
contract SymbolicRegressionSingle {
    function checkBoom(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--emit-regression", "--match-test", "checkBoom"])
        .assert_failure()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();
    assert!(stderr.contains("Regression test:"), "{stderr}");

    let regression = prj
        .root()
        .join("test/regressions/SymbolicRegressionSingle_checkBoom_SymbolicRegression.t.sol");
    assert!(regression.exists(), "missing regression {}", regression.display());

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--match-test", "test_regression_checkBoom_symbolic"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("test_regression_checkBoom_symbolic()"), "{stdout}");
    assert!(stdout.contains("assertion failed"), "{stdout}");
});

forgetest_init!(symbolic_emits_regression_for_contract_with_receive, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_regression_for_contract_with_receive because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRegressionReceive.t.sol",
        r#"
contract SymbolicRegressionReceive {
    receive() external payable {}

    function checkReceiveRegression(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--emit-regression", "--match-test", "checkReceiveRegression"])
        .assert_failure();

    let regression = prj.root().join(
        "test/regressions/SymbolicRegressionReceive_checkReceiveRegression_SymbolicRegression.t.sol",
    );
    let generated = std::fs::read_to_string(&regression)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", regression.display()));
    assert!(generated.contains("pragma solidity >=0.8.0;"), "{generated}");
    assert!(!generated.contains("receive() external payable"), "{generated}");

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--match-test", "test_regression_checkReceiveRegression_symbolic"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("test_regression_checkReceiveRegression_symbolic()"), "{stdout}");
    assert!(stdout.contains("assertion failed"), "{stdout}");
});

forgetest_init!(symbolic_emit_regression_rerun_reuses_existing_file, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emit_regression_rerun_reuses_existing_file because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRegressionRerun.t.sol",
        r#"
contract SymbolicRegressionRerun {
    function checkRerun(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    cmd.args(["test", "--symbolic", "--emit-regression", "--match-test", "checkRerun"])
        .assert_failure();

    let regression = prj
        .root()
        .join("test/regressions/SymbolicRegressionRerun_checkRerun_SymbolicRegression.t.sol");
    let generated = std::fs::read_to_string(&regression)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", regression.display()));

    let output = cmd
        .forge_fuse()
        .args(["test", "--symbolic", "--emit-regression", "--match-test", "checkRerun"])
        .assert_failure()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();
    assert!(!stderr.contains("already exists"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(&regression)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", regression.display())),
        generated
    );

    let nested = prj.root().join(
        "test/regressions/SymbolicRegressionRerun_checkRerun_SymbolicRegression_checkRerun_SymbolicRegression.t.sol",
    );
    assert!(!nested.exists(), "unexpected nested regression {}", nested.display());
});

forgetest_init!(symbolic_emits_regression_under_custom_test_dir, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_regression_under_custom_test_dir because z3 is not available"
        );
        return;
    }

    prj.update_config(|config| config.test = "tests".into());
    let tests_dir = prj.root().join("tests");
    std::fs::create_dir_all(&tests_dir).unwrap();
    std::fs::write(
        tests_dir.join("SymbolicRegressionCustomDir.t.sol"),
        format!(
            r#"// SPDX-License-Identifier: UNLICENSED
pragma solidity ={SOLC_VERSION};

contract SymbolicRegressionCustomDir {{
    function checkCustomDir(uint256 x) public pure {{
        assert(x != 42);
    }}
}}
"#
        ),
    )
    .unwrap();

    cmd.args(["test", "--symbolic", "--emit-regression", "--match-test", "checkCustomDir"])
        .assert_failure();

    let regression = prj.root().join(
        "tests/regressions/SymbolicRegressionCustomDir_checkCustomDir_SymbolicRegression.t.sol",
    );
    assert!(regression.exists(), "missing regression {}", regression.display());

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--match-test", "test_regression_checkCustomDir_symbolic"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("test_regression_checkCustomDir_symbolic()"), "{stdout}");
    assert!(stdout.contains("assertion failed"), "{stdout}");
});

forgetest_init!(symbolic_json_reports_solidity_regression, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_json_reports_solidity_regression because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRegressionJson.t.sol",
        r#"
contract SymbolicRegressionJson {
    function checkJsonRegression(uint256 x) public pure {
        assert(x != 42);
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--emit-regression",
            "--json",
            "--match-test",
            "checkJsonRegression",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "checkJsonRegression(uint256)");
    let regressions = result["symbolic_regressions"].as_array().expect("symbolic regressions");
    assert_eq!(regressions.len(), 1);
    assert!(regressions[0]["artifact"].is_string());
    let path = regressions[0]["path"].as_str().expect("regression path");
    assert!(path.ends_with(
        "test/regressions/SymbolicRegressionJson_checkJsonRegression_SymbolicRegression.t.sol"
    ));
    assert!(std::path::Path::new(path).exists());
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
    assert_eq!(result["counterexample_artifacts"].as_array().unwrap().len(), 2);
    assert_eq!(result["counterexample_artifacts"][0], symbolic["artifact"]);
    assert_eq!(symbolic["minimization"]["minimized"], symbolic["artifact"]);
    assert_eq!(symbolic["minimization"]["accepted"], 0);
    assert_eq!(symbolic["minimization"]["original_calldata_bytes"], 36);
    assert_eq!(symbolic["minimization"]["minimized_calldata_bytes"], 36);
    let original_artifact = read_artifact_ref(&symbolic["minimization"]["original"]);
    assert_eq!(original_artifact["calls"][0]["args"], "42");
    let artifact_path = symbolic["artifact"]["path"].as_str().unwrap().to_string();
    let artifact = read_artifact(symbolic);
    assert_eq!(artifact["schema_version"], 1);
    assert_eq!(artifact["schema"], "foundry:symbolic.counterexample@v1");
    assert_eq!(artifact["kind"], "single_call");
    assert_eq!(artifact["test"]["test"], "checkRejectsFortyTwo(uint256)");
    assert_eq!(artifact["replay"]["status"], "confirmed");
    assert_eq!(artifact["calls"].as_array().unwrap().len(), 1);
    assert_eq!(original_artifact["calls"][0]["sender"], artifact["calls"][0]["sender"]);
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
[FAIL:
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

forgetest_init!(symbolic_minimizes_replayed_counterexample_artifact, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_minimizes_replayed_counterexample_artifact because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMinimizeCounterexample.t.sol",
        r#"
contract SymbolicMinimizeCounterexample {
    /// forge-config: default.symbolic.array_lengths = [33]
    function checkMinimize(uint256 x, bytes memory data) public pure {
        if ((x & 0x2a) == 0x2a && data.length >= 2 && data[1] == 0x42) {
            assert(false);
        }
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkMinimize"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkMinimize(uint256,bytes)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(result["counterexample_artifacts"].as_array().unwrap().len(), 2);
    assert_eq!(symbolic["minimization"]["minimized"], symbolic["artifact"]);
    assert!(
        symbolic["minimization"]["attempts"].as_u64().unwrap()
            > symbolic["minimization"]["accepted"].as_u64().unwrap()
    );
    assert!(symbolic["minimization"]["accepted"].as_u64().unwrap() > 0);
    assert!(
        symbolic["minimization"]["minimized_calldata_bytes"].as_u64().unwrap()
            < symbolic["minimization"]["original_calldata_bytes"].as_u64().unwrap()
    );

    let original = read_artifact_ref(&symbolic["minimization"]["original"]);
    let minimized = read_artifact(symbolic);
    assert_eq!(original["replay"]["status"], "confirmed");
    assert_eq!(minimized["replay"]["status"], "confirmed");
    assert_ne!(original["calls"][0]["calldata"], minimized["calls"][0]["calldata"]);
    assert_eq!(minimized["calls"][0]["args"], "42, 0x0042");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            symbolic["artifact"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
args=[42, 0x0042]
"#]],
    );
});

forgetest_init!(symbolic_minimizer_skips_reasonless_failure_flag, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_minimizer_skips_reasonless_failure_flag because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMinimizeFailureFlag.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicMinimizeFailureFlag is Test {
    function checkFailureFlag(uint256 x) public {
        if (x == 0) revert("candidate-revert");
        if (x == 42) fail();
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkFailureFlag"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkFailureFlag(uint256)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(symbolic["replay"]["status"], "confirmed");
    assert_eq!(symbolic["counterexample"]["raw_args"], "42");
    assert!(symbolic["minimization"].is_null());
    assert_eq!(result["counterexample_artifacts"].as_array().unwrap().len(), 1);

    let artifact = read_artifact(symbolic);
    assert_eq!(artifact["replay"]["status"], "confirmed");
    assert_eq!(artifact["calls"][0]["raw_args"], "42");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            symbolic["artifact"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
args=[42]
"#]],
    );
});

forgetest_init!(symbolic_minimizes_echidna_address_array_duplicate_fixture, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_minimizes_echidna_address_array_duplicate_fixture because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicMinimizeAddressArrayDuplicate.t.sol",
        r#"
library AddressArrayUtilsBug {
    function hasDuplicate(address[] memory xs) internal pure returns (bool) {
        for (uint256 i = 0; i < xs.length; i++) {
            for (uint256 j = i + 1; j < xs.length; j++) {
                if (xs[i] == xs[j]) return true;
            }
        }
        return false;
    }
}

contract SymbolicMinimizeAddressArrayDuplicate {
    /// forge-config: default.symbolic.array_lengths = [6]
    function checkNoDuplicate(address[] memory xs) public pure {
        assert(!AddressArrayUtilsBug.hasDuplicate(xs));
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "checkNoDuplicate"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "checkNoDuplicate(address[])");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(result["counterexample_artifacts"].as_array().unwrap().len(), 2);
    assert!(symbolic["minimization"]["accepted"].as_u64().unwrap() > 0);
    assert_eq!(symbolic["minimization"]["original_calldata_bytes"], 260);
    assert_eq!(symbolic["minimization"]["minimized_calldata_bytes"], 132);

    let original = read_artifact_ref(&symbolic["minimization"]["original"]);
    let minimized = read_artifact(symbolic);
    assert_eq!(original["replay"]["status"], "confirmed");
    assert_eq!(minimized["replay"]["status"], "confirmed");
    assert_ne!(original["calls"][0]["calldata"], minimized["calls"][0]["calldata"]);
    assert_eq!(
        minimized["calls"][0]["args"],
        "[0x0000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000]"
    );

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            symbolic["artifact"]["path"].as_str().unwrap(),
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &replay_stdout,
        foundry_test_utils::str![[r#"
args=[[0x0000000000000000000000000000000000000000, 0x0000000000000000000000000000000000000000]]
"#]],
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
    let minimization = &failure["minimization"];
    assert_eq!(failure["artifact"], minimization["minimized"]);
    assert_eq!(minimization["minimized_sequence_len"], 1);
    assert!(
        minimization["original_sequence_len"].as_u64().unwrap()
            > minimization["minimized_sequence_len"].as_u64().unwrap()
    );
    let artifacts = result["counterexample_artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert_eq!(artifacts[0], failure["artifact"]);
    assert_eq!(artifacts[1], minimization["original"]);
    let artifact_path = failure["artifact"]["path"].as_str().unwrap().to_string();

    let original = read_artifact_ref(&minimization["original"]);
    assert_eq!(original["replay"]["status"], "confirmed");
    assert!(
        original["calls"].as_array().unwrap().len()
            > minimization["minimized_sequence_len"].as_u64().unwrap() as usize
    );
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

forgetest_init!(symbolic_emits_stateful_solidity_regression, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_stateful_solidity_regression because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRegressionSequence.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRegressionSequenceTarget {
    uint256 public value;

    function set(uint256 x) external {
        if (x == 7) {
            value = 11;
        }
    }
}

contract SymbolicRegressionSequence is Test {
    SymbolicRegressionSequenceTarget target;

    function setUp() public {
        target = new SymbolicRegressionSequenceTarget();
        targetContract(address(target));
    }

    function invariant_counterNeverEleven() public view {
        assert(target.value() != 11);
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--emit-regression",
            "--match-test",
            "invariant_counterNeverEleven",
        ])
        .assert_failure()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();
    assert!(stderr.contains("Regression test:"), "{stderr}");

    let regression = prj.root().join(
        "test/regressions/SymbolicRegressionSequence_invariant_counterNeverEleven_SymbolicRegression.t.sol",
    );
    assert!(regression.exists(), "missing regression {}", regression.display());

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--match-test", "test_regression_invariant_counterNeverEleven_symbolic"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("test_regression_invariant_counterNeverEleven_symbolic()"), "{stdout}");
    assert!(stdout.contains("assertion failed"), "{stdout}");
});

forgetest_init!(symbolic_emits_stateful_regression_with_after_invariant, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_stateful_regression_with_after_invariant because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicRegressionAfterInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRegressionAfterInvariantTarget {
    uint256 public value;

    function set(uint256 x) external {
        if (x == 7) {
            value = 1;
        }
    }
}

contract SymbolicRegressionAfterInvariant is Test {
    SymbolicRegressionAfterInvariantTarget target;

    function setUp() public {
        target = new SymbolicRegressionAfterInvariantTarget();
        targetContract(address(target));
    }

    function invariant_ok() public pure {}

    function afterInvariant() public view {
        require(target.value() != 1, "afterInvariant failure");
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--emit-regression",
            "--match-test",
            "invariant_ok",
            "--symbolic-invariant-depth",
            "1",
        ])
        .assert_failure()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();
    assert!(stderr.contains("Regression test:"), "{stderr}");

    let regression = prj.root().join(
        "test/regressions/SymbolicRegressionAfterInvariant_invariant_ok_SymbolicRegression.t.sol",
    );
    assert!(regression.exists(), "missing regression {}", regression.display());
    let generated = std::fs::read_to_string(&regression)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", regression.display()));
    assert!(generated.contains(r#"hex"93969ddf""#), "{generated}");

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--match-test", "test_regression_invariant_ok_symbolic"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("test_regression_invariant_ok_symbolic()"), "{stdout}");
    assert!(stdout.contains("afterInvariant failure"), "{stdout}");
});

forgetest_init!(symbolic_emits_handler_assertion_regression, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_handler_assertion_regression because z3 is not available"
        );
        return;
    }

    prj.update_config(|config| {
        config.invariant.fail_on_revert = false;
    });
    prj.add_test(
        "SymbolicRegressionHandler.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRegressionHandlerTarget {
    uint256 sink;

    function boom(uint256 x) external {
        sink = x;
        if (x == 7) {
            assert(false);
        }
    }
}

contract SymbolicRegressionHandler is Test {
    SymbolicRegressionHandlerTarget target;

    function setUp() public {
        target = new SymbolicRegressionHandlerTarget();
        targetContract(address(target));
    }

    function invariant_ok() public pure {}
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--emit-regression",
            "--match-test",
            "invariant_ok",
            "--symbolic-invariant-depth",
            "1",
        ])
        .assert_failure()
        .get_output()
        .clone();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();
    assert!(stdout.contains("Assertion Tests: 1 assertion bug(s) found"), "{stdout}");
    assert!(stderr.contains("Regression test:"), "{stderr}");

    let regression = prj
        .root()
        .join("test/regressions/SymbolicRegressionHandler_invariant_ok_SymbolicRegression.t.sol");
    assert!(regression.exists(), "missing regression {}", regression.display());
    let generated = std::fs::read_to_string(&regression)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", regression.display()));
    assert!(generated.contains("true);"), "{generated}");

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--match-test", "test_regression_invariant_ok_symbolic"])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("test_regression_invariant_ok_symbolic()"), "{stdout}");
    assert!(stdout.contains("assertion failed"), "{stdout}");
});

forgetest_init!(symbolic_emits_multiple_handler_assertion_regressions, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_emits_multiple_handler_assertion_regressions because z3 is not available"
        );
        return;
    }

    prj.update_config(|config| {
        config.invariant.fail_on_revert = false;
    });
    prj.add_test(
        "SymbolicRegressionMultipleHandlers.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicRegressionMultipleHandlersFirst {
    uint256 sink;

    function boomFirst(uint256 x) external {
        sink = x;
        if (x == 7) {
            assert(false);
        }
    }
}

contract SymbolicRegressionMultipleHandlersSecond {
    uint256 sink;

    function boomSecond(uint256 x) external {
        sink = x;
        if (x == 11) {
            assert(false);
        }
    }
}

contract SymbolicRegressionMultipleHandlers is Test {
    SymbolicRegressionMultipleHandlersFirst first;
    SymbolicRegressionMultipleHandlersSecond second;

    function setUp() public {
        first = new SymbolicRegressionMultipleHandlersFirst();
        second = new SymbolicRegressionMultipleHandlersSecond();
        targetContract(address(first));
        targetContract(address(second));
    }

    function invariant_ok() public pure {}
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--emit-regression",
            "--match-test",
            "invariant_ok",
            "--symbolic-invariant-depth",
            "1",
        ])
        .assert_failure()
        .get_output()
        .clone();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();
    assert!(stdout.contains("Assertion Tests: 2 assertion bug(s) found"), "{stdout}");
    assert_eq!(stderr.matches("Regression test:").count(), 2, "{stderr}");

    let regressions_dir = prj.root().join("test/regressions");
    let regressions = std::fs::read_dir(&regressions_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", regressions_dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sol"))
        .collect::<Vec<_>>();
    assert_eq!(regressions.len(), 2, "{regressions:?}");
    assert!(
        regressions.iter().any(|path| path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("_handler_")),
        "{regressions:?}"
    );
});

forgetest_init!(symbolic_json_reports_minimized_sequence_counterexample, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_json_reports_minimized_sequence_counterexample because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInvariantSequenceMinimize.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicSequenceMinTarget {
    bool primed;
    bool fired;

    function prime(uint256 x) external {
        if (x > 40) {
            primed = true;
        }
    }

    function fire(uint256 y) external {
        if (y > 100) {
            fired = true;
        }
    }

    function broken() external view returns (bool) {
        return primed && fired;
    }
}

contract SymbolicInvariantSequenceMinimize is Test {
    SymbolicSequenceMinTarget target;
    uint256[] public fixture_x = [1000];
    uint256[] public fixture_y = [5000];

    function setUp() public {
        target = new SymbolicSequenceMinTarget();
        bytes4[] memory selectors = new bytes4[](2);
        selectors[0] = SymbolicSequenceMinTarget.prime.selector;
        selectors[1] = SymbolicSequenceMinTarget.fire.selector;
        targetSelector(FuzzSelector({addr: address(target), selectors: selectors}));
    }

    /// forge-config: default.invariant.runs = 1
    /// forge-config: default.invariant.depth = 20
    /// forge-config: default.invariant.check_interval = 0
    /// forge-config: default.invariant.shrink_run_limit = 5000
    function invariant_targetNeverBroken() public view {
        assertEq(target.broken(), false);
    }
}
"#,
    );

    let output = cmd
        .args([
            "test",
            "--symbolic",
            "--json",
            "--match-test",
            "invariant_targetNeverBroken",
            "--fuzz-seed",
            "1",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();

    let result = json_test_result(&output, "invariant_targetNeverBroken()");
    let failures = result["invariant_failures"].as_array().expect("invariant failures");
    let failure = failures.first().expect("invariant failure");
    let minimization = &failure["minimization"];
    assert_eq!(failure["artifact"], minimization["minimized"]);
    assert_eq!(minimization["original_sequence_len"], 20);
    assert_eq!(minimization["minimized_sequence_len"], 2);
    assert!(minimization["accepted"].as_u64().unwrap() > 0);
    assert!(
        minimization["minimized_calldata_bytes"].as_u64().unwrap()
            < minimization["original_calldata_bytes"].as_u64().unwrap()
    );
    let artifacts = result["counterexample_artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert!(artifacts.contains(&failure["artifact"]));
    assert!(artifacts.contains(&minimization["original"]));

    let original = read_artifact_ref(&minimization["original"]);
    let minimized = read_artifact_ref(&minimization["minimized"]);
    assert_eq!(original["replay"]["status"], "confirmed");
    assert_eq!(minimized["replay"]["status"], "confirmed");
    assert_eq!(original["calls"].as_array().unwrap().len(), 20);
    assert_eq!(minimized["calls"].as_array().unwrap().len(), 2);

    let calls = minimized["calls"].as_array().unwrap();
    let prime = calls.iter().find(|call| call["function_name"] == "prime").unwrap();
    let fire = calls.iter().find(|call| call["function_name"] == "fire").unwrap();
    assert_eq!(prime["args"], "41");
    assert_eq!(fire["args"], "101");

    let replay_stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--replay-symbolic-artifact",
            minimization["minimized"]["path"].as_str().unwrap(),
        ])
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
invariant_targetNeverBroken()
"#]],
    );
});

forgetest_init!(symbolic_seed_corpus_persists_non_failing_fuzz_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_seed_corpus_persists_non_failing_fuzz_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFuzzCorpusSeed.t.sol",
        r#"
contract SymbolicFuzzCorpusSeed {
    function testHybridFindsBug(uint256 x, uint256 y) public pure {
        unchecked {
            if (x * 7 != 1) return;
        }
        if (y != 0) return;
        assert(y == 0);
    }
}
"#,
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testHybridFindsBug",
            "--symbolic-seed-corpus",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
        ])
        .assert_success();

    let corpus_dir = prj
        .root()
        .join("fuzz_corpus")
        .join("SymbolicFuzzCorpusSeed")
        .join("testHybridFindsBug")
        .join("worker0")
        .join("corpus");
    let mut entries = std::fs::read_dir(&corpus_dir)
        .unwrap_or_else(|err| panic!("failed to read corpus dir {}: {err}", corpus_dir.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(|entry| entry.path());
    let expected_selector = &keccak256(b"testHybridFindsBug(uint256,uint256)")[..4];
    let expected_x = "6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db6db7";
    let expected_y = format!("{:064x}", 0);
    let expected_x_decimal =
        "49625181101706940895816136432294817651401421999560241731196107431962769845687";
    let mut found_symbolic_seed = false;
    for entry in &entries {
        let corpus: Value = serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
        assert_eq!(corpus.as_array().unwrap().len(), 1);
        let calldata = corpus[0]["calldata"].as_str().expect("seed calldata");
        assert_eq!(&hex::decode(&calldata[2..10]).unwrap(), expected_selector);
        if calldata[10..74] == *expected_x && calldata[74..138] == expected_y {
            found_symbolic_seed = true;
        } else {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    assert!(found_symbolic_seed);

    prj.add_test(
        "SymbolicFuzzCorpusSeed.t.sol",
        r#"
contract SymbolicFuzzCorpusSeed {
    function testHybridFindsBug(uint256 x, uint256 y) public pure {
        unchecked {
            if (x * 7 != 1) return;
        }
        if (y == 0) return;
        assert(false);
    }

    function checkHybridFindsBug(uint256 x, uint256 y) public pure {
        unchecked {
            if (x * 7 != 1) return;
        }
        if (y != 0) return;
        assert(y == 0);
    }
}
"#,
    );

    cmd.forge_fuse()
        .args(["test", "--match-test", "testHybridFindsBug", "--threads", "1", "--fuzz-runs", "8"])
        .assert_success();

    assert_symbolic(cmd.forge_fuse().args([
        "test",
        "--symbolic",
        "--match-test",
        "checkHybridFindsBug",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicFuzzCorpusSeed.t.sol:SymbolicFuzzCorpusSeed
[PASS] checkHybridFindsBug(uint256,uint256) ([METRICS])
...
"#]]);

    let stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testHybridFindsBug",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--threads",
            "1",
            "--fuzz-corpus-random-sequence-weight",
            "0",
            "--fuzz-mutation-weight-splice",
            "0",
            "--fuzz-mutation-weight-repeat",
            "0",
            "--fuzz-mutation-weight-interleave",
            "0",
            "--fuzz-mutation-weight-prefix",
            "0",
            "--fuzz-mutation-weight-suffix",
            "0",
            "--fuzz-mutation-weight-abi",
            "1",
            "--fuzz-mutation-weight-cmp",
            "0",
            "--fuzz-runs",
            "256",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains(expected_x_decimal), "{stdout}");
});

forgetest_init!(symbolic_seed_corpus_is_best_effort_for_symbolic_incomplete, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_seed_corpus_is_best_effort_for_symbolic_incomplete because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFuzzCorpusBestEffort.t.sol",
        r#"
contract SymbolicFuzzCorpusBestEffort {
    function testFuzz_symbolicHostile(uint256) public pure {
        for (uint256 i; i < 10001; ++i) {}
    }
}
"#,
    );

    let stdout = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_symbolicHostile",
            "--symbolic-seed-corpus",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--fuzz-runs",
            "1",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("[PASS] testFuzz_symbolicHostile(uint256)"), "{stdout}");
});

forgetest_init!(symbolic_fuzz_frontier_seeding_persists_branch_flipping_input, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_fuzz_frontier_seeding_persists_branch_flipping_input because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFuzzFrontierSeed.t.sol",
        r#"
contract SymbolicFuzzFrontierSeed {
    function testFuzz_frontier(uint64 amount, uint256 feeMultiplier) public pure {
        uint256 credited;
        unchecked {
            credited = uint256(amount) + (feeMultiplier - 100);
        }

        if (feeMultiplier < 100) {
            assert(credited <= amount);
        }
    }
}
"#,
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_frontier",
            "--fuzz-runs",
            "8",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
        ])
        .assert_success();

    let target_frontier = matching_frontier(
        prj.root(),
        "SymbolicFuzzFrontierSeed",
        "testFuzz_frontier",
        "missed fee multiplier",
        |frontier| {
            frontier["site"]["opcode_name"] == "LT"
                && (frontier["operands"]["lhs"] == "0x64" || frontier["operands"]["rhs"] == "0x64")
        },
    );
    let target_frontier_id = target_frontier["id"].as_u64().unwrap().to_string();
    let target_frontier_pc = target_frontier["site"]["pc"].as_u64().unwrap().to_string();
    let target_frontier_selector =
        target_frontier["sequence"][0]["calldata"].as_str().unwrap()[..10].to_string();

    let output = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_frontier",
            "--fuzz-runs",
            "1",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-use-fuzz-frontiers",
            "--symbolic-frontier-limit",
            "1",
            "--symbolic-frontier-ids",
            &target_frontier_id,
            "--symbolic-frontier-pcs",
            &target_frontier_pc,
            "--symbolic-frontier-selectors",
            &target_frontier_selector,
        ])
        .assert_success()
        .get_output()
        .clone();
    let stdout = output.stdout_lossy();
    let stderr = output.stderr_lossy();
    assert!(stdout.contains("testFuzz_frontier(uint64,uint256)"), "{stdout}");
    assert!(
        stderr.contains("Symbolic frontier selection for testFuzz_frontier(uint64,uint256)"),
        "{stderr}"
    );

    let corpus_dir = prj
        .root()
        .join("fuzz_corpus")
        .join("SymbolicFuzzFrontierSeed")
        .join("testFuzz_frontier")
        .join("worker0")
        .join("corpus");
    let expected_selector = hex::encode(&keccak256(b"testFuzz_frontier(uint64,uint256)")[..4]);
    let mut found_branch_flipping_seed = false;
    for entry in std::fs::read_dir(&corpus_dir)
        .unwrap_or_else(|err| panic!("failed to read corpus dir {}: {err}", corpus_dir.display()))
    {
        let entry = entry.unwrap();
        let corpus: Value = serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
        for tx in corpus.as_array().unwrap() {
            let calldata = tx["calldata"].as_str().expect("seed calldata");
            if !calldata.starts_with(&format!("0x{expected_selector}")) {
                continue;
            }
            let fee_multiplier = U256::from_be_slice(&hex::decode(&calldata[74..138]).unwrap());
            if fee_multiplier < U256::from(100) {
                found_branch_flipping_seed = true;
            }
        }
    }
    assert!(found_branch_flipping_seed);

    let replay_output = cmd
        .forge_fuse()
        .args([
            "fuzz",
            "replay",
            "--match-contract",
            "SymbolicFuzzFrontierSeed",
            "--match-test",
            "testFuzz_frontier",
            "--corpus-dir",
            "fuzz_corpus",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(replay_output.contains("corpus replay failed"), "{replay_output}");
});

forgetest_init!(symbolic_fuzz_frontier_seeding_ignores_pre_target_counterexamples, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_fuzz_frontier_seeding_ignores_pre_target_counterexamples because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFuzzFrontierTargetGate.t.sol",
        r#"
/// forge-config: default.symbolic.exploration_order = "dfs"
contract SymbolicFuzzFrontierTargetGate {
    event TargetHit();

    function testFuzz_targetGate(uint256 value) public {
        if (value == 13) {
            assert(false);
        }

        if (value < 777) {
            emit TargetHit();
        }
    }
}
"#,
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_targetGate",
            "--fuzz-runs",
            "1",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
        ])
        .assert_success();

    keep_only_matching_frontier(
        prj.root(),
        "SymbolicFuzzFrontierTargetGate",
        "testFuzz_targetGate",
        "value < 777",
        |frontier| {
            frontier["site"]["opcode_name"] == "LT"
                && (frontier["operands"]["lhs"] == "0x309"
                    || frontier["operands"]["rhs"] == "0x309")
        },
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_targetGate",
            "--fuzz-runs",
            "1",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-use-fuzz-frontiers",
            "--symbolic-frontier-limit",
            "1",
        ])
        .assert_success();

    let values = single_uint_corpus_values(
        prj.root(),
        "SymbolicFuzzFrontierTargetGate",
        "testFuzz_targetGate",
        "testFuzz_targetGate(uint256)",
    );
    assert!(values.iter().any(|value| *value < U256::from(777)));
    assert!(!values.iter().any(|value| *value == U256::from(13)));
});

forgetest_init!(symbolic_fuzz_frontier_seeding_keeps_deploy_code_target_progress, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_fuzz_frontier_seeding_keeps_deploy_code_target_progress because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFuzzDeployCodeFrontier.t.sol",
        r#"
import "forge-std/Test.sol";

interface SymbolicDeployCodeVm {
    function randomUint(uint256 min, uint256 max) external view returns (uint256);
}

contract DeployCodeFrontierCtor {
    SymbolicDeployCodeVm constant VM =
        SymbolicDeployCodeVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    event TargetHit();

    constructor() {
        uint256 value = VM.randomUint(0, 1000);
        if (value < 777) {
            emit TargetHit();
        }
    }
}

contract SymbolicFuzzDeployCodeFrontier is Test {
    string constant TARGET = "test/SymbolicFuzzDeployCodeFrontier.t.sol";

    function testFuzz_deployCode(uint256 marker) public {
        marker;
        vm.deployCode(string.concat(TARGET, ":DeployCodeFrontierCtor"));
    }
}
"#,
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_deployCode",
            "--fuzz-runs",
            "1",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
        ])
        .assert_success();

    let target_frontier = keep_only_matching_frontier(
        prj.root(),
        "SymbolicFuzzDeployCodeFrontier",
        "testFuzz_deployCode",
        "deployCode constructor value < 777",
        |frontier| {
            frontier["site"]["opcode_name"] == "LT"
                && (frontier["operands"]["lhs"] == "0x309"
                    || frontier["operands"]["rhs"] == "0x309")
        },
    );
    let frontier_calldata = target_frontier["sequence"][0]["calldata"].as_str().unwrap();
    let frontier_marker = U256::from_be_slice(&hex::decode(&frontier_calldata[10..74]).unwrap());
    assert_ne!(frontier_marker, U256::ZERO);

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_deployCode",
            "--fuzz-runs",
            "1",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-use-fuzz-frontiers",
            "--symbolic-frontier-limit",
            "1",
        ])
        .assert_success();

    let values = single_uint_corpus_values(
        prj.root(),
        "SymbolicFuzzDeployCodeFrontier",
        "testFuzz_deployCode",
        "testFuzz_deployCode(uint256)",
    );
    assert!(values.contains(&U256::ZERO), "target_frontier={target_frontier}, values={values:?}");
});

forgetest_init!(symbolic_fuzz_frontier_seeding_keeps_callee_target_progress, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_fuzz_frontier_seeding_keeps_callee_target_progress because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicFuzzCalleeFrontierSeed.t.sol",
        r#"
contract SymbolicFuzzCalleeTarget {
    function crossed(uint256 value) external pure returns (bool) {
        return value == 777;
    }
}

contract SymbolicFuzzCalleeFrontierSeed {
    SymbolicFuzzCalleeTarget target = new SymbolicFuzzCalleeTarget();

    function testFuzz_callee(uint256 value) public view {
        target.crossed(value);
    }
}
"#,
    );

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_callee",
            "--fuzz-runs",
            "8",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
        ])
        .assert_success();

    cmd.forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_callee",
            "--fuzz-runs",
            "1",
            "--fuzz-seed",
            "0x1234",
            "--threads",
            "1",
            "--fuzz-frontier-dir",
            "fuzz_frontiers",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-use-fuzz-frontiers",
            "--symbolic-frontier-limit",
            "32",
        ])
        .assert_success();

    let corpus_dir = prj
        .root()
        .join("fuzz_corpus")
        .join("SymbolicFuzzCalleeFrontierSeed")
        .join("testFuzz_callee")
        .join("worker0")
        .join("corpus");
    let expected_selector = hex::encode(&keccak256(b"testFuzz_callee(uint256)")[..4]);
    let mut found_branch_flipping_seed = false;
    for entry in std::fs::read_dir(&corpus_dir)
        .unwrap_or_else(|err| panic!("failed to read corpus dir {}: {err}", corpus_dir.display()))
    {
        let entry = entry.unwrap();
        let corpus: Value = serde_json::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
        for tx in corpus.as_array().unwrap() {
            let calldata = tx["calldata"].as_str().expect("seed calldata");
            if !calldata.starts_with(&format!("0x{expected_selector}")) {
                continue;
            }
            let value = U256::from_be_slice(&hex::decode(&calldata[10..74]).unwrap());
            if value == U256::from(777) {
                found_branch_flipping_seed = true;
            }
        }
    }
    assert!(found_branch_flipping_seed);
});

forgetest_init!(symbolic_import_fuzz_corpus_guides_bounded_symbolic_path, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_import_fuzz_corpus_guides_bounded_symbolic_path because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicImportFuzzCorpus.t.sol",
        r#"
contract SymbolicImportFuzzCorpus {
    function testGuided(uint256 x) public pure {
        if (x != 7) return;
        assert(false);
    }
}
"#,
    );

    let empty_output = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testGuided",
            "--symbolic-use-fuzz-corpus",
            "--fuzz-corpus-dir",
            "empty_fuzz_corpus",
            "--symbolic-width",
            "1",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let empty_result = json_test_result(&empty_output, "testGuided(uint256)");
    let empty_symbolic = &empty_result["symbolic"];
    assert_eq!(empty_symbolic["status"], "incomplete");
    assert_eq!(empty_symbolic["corpus_seeds"]["used"].as_array().unwrap().len(), 0);

    let selector = &keccak256(b"testGuided(uint256)")[..4];
    let calldata = format!("0x{}{:064x}", hex::encode(selector), 7);
    let corpus_dir = prj
        .root()
        .join("fuzz_corpus")
        .join("SymbolicImportFuzzCorpus")
        .join("testGuided")
        .join("worker0")
        .join("corpus");
    std::fs::create_dir_all(&corpus_dir).unwrap();
    let seed_path = corpus_dir.join("00000000-0000-0000-0000-000000000001-1.json");
    let seed = serde_json::json!([
        {
            "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
            "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
            "calldata": calldata
        }
    ]);
    std::fs::write(&seed_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();

    let output = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testGuided",
            "--symbolic-use-fuzz-corpus",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-width",
            "1",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "testGuided(uint256)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(symbolic["counterexample"]["raw_args"], "7");
    assert_eq!(symbolic["corpus_seeds"]["loaded"], 1);
    assert_eq!(symbolic["corpus_seeds"]["skipped"], 0);
    let used = symbolic["corpus_seeds"]["used"].as_array().unwrap();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0]["calldata"], calldata);
    assert_eq!(std::path::PathBuf::from(used[0]["path"].as_str().unwrap()), seed_path);
});

forgetest_init!(symbolic_import_fuzz_corpus_prioritizes_seeded_calldata_variant, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_import_fuzz_corpus_prioritizes_seeded_calldata_variant because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicImportFuzzCorpusVariants.t.sol",
        r#"
contract SymbolicImportFuzzCorpusVariants {
    /// forge-config: default.symbolic.default_bytes_lengths = [1, 2]
    function testGuidedBytes(bytes memory data) public pure {
        if (data.length == 1) {
            if (data[0] == 0x11) return;
            return;
        }
        if (data.length != 2) return;
        assert(false);
    }
}
"#,
    );

    let selector = &keccak256(b"testGuidedBytes(bytes)")[..4];
    let calldata = format!(
        "0x{}{:064x}{:064x}{:0<64}",
        hex::encode(selector),
        32,
        2,
        hex::encode([0xaa, 0xbb])
    );
    let unmodeled_calldata = format!(
        "0x{}{:064x}{:064x}{:0<64}",
        hex::encode(selector),
        32,
        3,
        hex::encode([0xcc, 0xdd, 0xee])
    );
    let corpus_dir = prj
        .root()
        .join("fuzz_corpus")
        .join("SymbolicImportFuzzCorpusVariants")
        .join("testGuidedBytes")
        .join("worker0")
        .join("corpus");
    std::fs::create_dir_all(&corpus_dir).unwrap();
    let seed_path = corpus_dir.join("00000000-0000-0000-0000-000000000001-1.json");
    let unmodeled_seed_path = corpus_dir.join("00000000-0000-0000-0000-000000000002-1.json");
    for (path, calldata) in
        [(&seed_path, calldata.as_str()), (&unmodeled_seed_path, unmodeled_calldata.as_str())]
    {
        let seed = serde_json::json!([
            {
                "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
                "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
                "calldata": calldata
            }
        ]);
        std::fs::write(path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();
    }

    let output = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testGuidedBytes",
            "--symbolic-use-fuzz-corpus",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-width",
            "2",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "testGuidedBytes(bytes)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(symbolic["corpus_seeds"]["loaded"], 2);
    assert_eq!(symbolic["corpus_seeds"]["skipped"], 0);
    let used = symbolic["corpus_seeds"]["used"].as_array().unwrap();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0]["calldata"], calldata);
    assert_eq!(std::path::PathBuf::from(used[0]["path"].as_str().unwrap()), seed_path);
});

forgetest_init!(symbolic_import_fuzz_corpus_honors_function_inline_config, |prj, cmd| {
    if !z3_available() {
        let _ = sh_eprintln!(
            "skipping symbolic_import_fuzz_corpus_honors_function_inline_config because z3 is not available"
        );
        return;
    }

    prj.add_test(
        "SymbolicInlineImportFuzzCorpus.t.sol",
        r#"
contract SymbolicInlineImportFuzzCorpus {
    /// forge-config: default.symbolic.use_fuzz_corpus = true
    function testFuzz_inline(uint256 x) public pure {
        if (x != 7) return;
        assert(false);
    }
}
"#,
    );

    let selector = &keccak256(b"testFuzz_inline(uint256)")[..4];
    let calldata = format!("0x{}{:064x}", hex::encode(selector), 7);
    let corpus_dir = prj
        .root()
        .join("fuzz_corpus")
        .join("SymbolicInlineImportFuzzCorpus")
        .join("testFuzz_inline")
        .join("worker0")
        .join("corpus");
    std::fs::create_dir_all(&corpus_dir).unwrap();
    let seed_path = corpus_dir.join("00000000-0000-0000-0000-000000000001-1.json");
    let seed = serde_json::json!([
        {
            "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
            "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
            "calldata": calldata
        }
    ]);
    std::fs::write(&seed_path, serde_json::to_vec_pretty(&seed).unwrap()).unwrap();

    let output = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_inline",
            "--fuzz-corpus-dir",
            "fuzz_corpus",
            "--symbolic-width",
            "1",
            "--json",
        ])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "testFuzz_inline(uint256)");
    let symbolic = &result["symbolic"];
    assert_eq!(symbolic["status"], "fail_counterexample");
    assert_eq!(symbolic["counterexample"]["raw_args"], "7");
    assert_eq!(symbolic["corpus_seeds"]["loaded"], 1);
    let used = symbolic["corpus_seeds"]["used"].as_array().unwrap();
    assert_eq!(used.len(), 1);
    assert_eq!(used[0]["calldata"], calldata);
});

forgetest_init!(symbolic_seed_corpus_warns_without_corpus_dir, |prj, cmd| {
    prj.add_test(
        "SymbolicFuzzCorpusNoDir.t.sol",
        r#"
contract SymbolicFuzzCorpusNoDir {
    function testFuzz_noop(uint256) public pure {}
}
"#,
    );

    let output = cmd
        .forge_fuse()
        .args([
            "test",
            "--match-test",
            "testFuzz_noop",
            "--symbolic-seed-corpus",
            "--fuzz-runs",
            "1",
        ])
        .assert_success()
        .get_output()
        .clone();
    let stderr = output.stderr_lossy();

    assert!(
        stderr
            .contains("`--symbolic-seed-corpus` requires `--fuzz-corpus-dir` or `fuzz.corpus_dir`"),
        "{stderr}"
    );
});

forgetest_init!(symbolic_artifact_replay_uses_stored_fail_on_revert, |prj, cmd| {
    prj.update_config(|config| {
        config.invariant.fail_on_revert = false;
    });
    prj.add_test(
        "SymbolicArtifactFailOnRevert.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicArtifactFailOnRevert is Test {
    uint256 ignored;

    function setUp() public {
        targetContract(address(this));
    }

    function step() external {
        ignored = 1;
        revert("boom");
    }

    function skip() external {
        vm.assume(false);
    }

    function invariant_noop() public pure {}
}
"#,
    );

    let artifact_path = prj.root().join("fail-on-revert-artifact.json");
    let artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "sequence",
        "test": {
            "contract": "test/SymbolicArtifactFailOnRevert.t.sol:SymbolicArtifactFailOnRevert",
            "test": "invariant_noop()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": "boom"
        },
        "replay_semantics": {
            "fail_on_revert": true
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 1,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [
            {
                "warp": null,
                "roll": null,
                "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
                "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
                "calldata": "0x1d2aa5b3",
                "value": null,
                "contract_name": "SymbolicArtifactFailOnRevert",
                "function_name": "skip",
                "signature": "skip()",
                "args": "",
                "raw_args": ""
            },
            {
                "warp": null,
                "roll": null,
                "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
                "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
                "calldata": "0xe25fe175",
                "value": null,
                "contract_name": "SymbolicArtifactFailOnRevert",
                "function_name": "step",
                "signature": "step()",
                "args": "",
                "raw_args": ""
            }
        ]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let mut bare_contract_artifact = artifact.clone();
    bare_contract_artifact["test"]["contract"] = serde_json::json!("SymbolicArtifactFailOnRevert");
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&bare_contract_artifact).unwrap())
        .unwrap();
    let stderr = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(stderr.contains("test.contract must be `path:Contract`"), "{stderr}");
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let output = cmd
        .forge_fuse()
        .args(["test", "--json", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_noop()");
    assert_eq!(result["status"], "Failure");
    assert!(result["kind"].get("Invariant").is_some(), "{result}");
    assert_eq!(result["kind"]["Invariant"]["runs"], 1);
    assert_eq!(result["kind"]["Invariant"]["calls"], 2);
    assert_eq!(result["kind"]["Invariant"]["reverts"], 1);
    assert!(result["kind"].get("Unit").is_none(), "{result}");

    let fixed_artifact_path = prj.root().join("fixed-artifact.json");
    let mut fixed_artifact = artifact;
    fixed_artifact["replay_semantics"]["fail_on_revert"] = serde_json::json!(false);
    std::fs::write(&fixed_artifact_path, serde_json::to_vec_pretty(&fixed_artifact).unwrap())
        .unwrap();

    let output = cmd
        .forge_fuse()
        .args([
            "test",
            "--json",
            "--replay-symbolic-artifact",
            fixed_artifact_path.to_str().unwrap(),
        ])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_noop()");
    assert_eq!(result["status"], "Success");
    assert!(result["kind"].get("Invariant").is_some(), "{result}");
    assert_eq!(result["kind"]["Invariant"]["runs"], 1);
    assert_eq!(result["kind"]["Invariant"]["calls"], 2);
    assert_eq!(result["kind"]["Invariant"]["reverts"], 1);
    assert!(result["kind"].get("Unit").is_none(), "{result}");
});

forgetest_init!(symbolic_artifact_replay_matches_bracketed_path, |prj, cmd| {
    prj.add_test(
        "SymbolicArtifact[Replay].t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicArtifactBracketPath is Test {
    function setUp() public {
        targetContract(address(this));
    }

    function step() external {
        revert("boom");
    }

    function invariant_noop() public pure {}
}
"#,
    );

    let artifact_path = prj.root().join("bracketed-path-artifact.json");
    let artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "sequence",
        "test": {
            "contract": "test/SymbolicArtifact[Replay].t.sol:SymbolicArtifactBracketPath",
            "test": "invariant_noop()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": "boom"
        },
        "replay_semantics": {
            "fail_on_revert": true
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 1,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [
            {
                "warp": null,
                "roll": null,
                "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
                "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
                "calldata": "0xe25fe175",
                "value": null,
                "contract_name": "SymbolicArtifactBracketPath",
                "function_name": "step",
                "signature": "step()",
                "args": "",
                "raw_args": ""
            }
        ]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("invariant_noop()"), "{stdout}");
    assert!(stdout.contains("boom"), "{stdout}");
});

forgetest_init!(symbolic_artifact_replay_rejects_stale_sequence_target, |prj, cmd| {
    prj.add_test(
        "SymbolicArtifactStaleTarget.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicArtifactStaleTarget is Test {
    function setUp() public {
        targetContract(address(this));
    }

    function step() external {}

    function invariant_noop() public pure {}
}
"#,
    );

    let artifact_path = prj.root().join("stale-sequence-selector-artifact.json");
    let mut artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "sequence",
        "test": {
            "contract": "test/SymbolicArtifactStaleTarget.t.sol:SymbolicArtifactStaleTarget",
            "test": "invariant_noop()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": null
        },
        "replay_semantics": {
            "fail_on_revert": false
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 1,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [{
            "warp": null,
            "roll": null,
            "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
            "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
            "calldata": "0xffffffff",
            "value": null,
            "contract_name": "SymbolicArtifactStaleTarget",
            "function_name": "step",
            "signature": "step()",
            "args": "",
            "raw_args": ""
        }]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("targets unknown function"), "{stdout}");

    let stale_target_artifact_path = prj.root().join("stale-sequence-target-artifact.json");
    artifact["calls"][0]["target"] =
        serde_json::json!("0x0000000000000000000000000000000000000000");
    artifact["calls"][0]["calldata"] = serde_json::json!("0xe25fe175");
    std::fs::write(&stale_target_artifact_path, serde_json::to_vec_pretty(&artifact).unwrap())
        .unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", stale_target_artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("targets unknown function"), "{stdout}");
});

forgetest_init!(symbolic_artifact_replay_rejects_forbidden_sequence_sender, |prj, cmd| {
    prj.add_test(
        "SymbolicArtifactForbiddenSender.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicArtifactForbiddenSender is Test {
    bool drained;
    address constant BOB = address(0xB0B);

    function setUp() public {
        targetContract(address(this));
        excludeSender(BOB);
    }

    function step() external {
        if (msg.sender == BOB) {
            drained = true;
        }
    }

    function invariant_notDrained() public view {
        assert(!drained);
    }
}
"#,
    );

    let artifact_path = prj.root().join("forbidden-sequence-sender-artifact.json");
    let artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "sequence",
        "test": {
            "contract": "test/SymbolicArtifactForbiddenSender.t.sol:SymbolicArtifactForbiddenSender",
            "test": "invariant_notDrained()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": null
        },
        "replay_semantics": {
            "fail_on_revert": false
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 1,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [{
            "warp": null,
            "roll": null,
            "sender": "0x0000000000000000000000000000000000000b0b",
            "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
            "calldata": "0xe25fe175",
            "value": null,
            "contract_name": "SymbolicArtifactForbiddenSender",
            "function_name": "step",
            "signature": "step()",
            "args": "",
            "raw_args": ""
        }]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("uses forbidden sender"), "{stdout}");

    let zero_sender_artifact_path = prj.root().join("zero-sequence-sender-artifact.json");
    let mut zero_sender_artifact = artifact;
    zero_sender_artifact["calls"][0]["sender"] =
        serde_json::json!("0x0000000000000000000000000000000000000000");
    std::fs::write(
        &zero_sender_artifact_path,
        serde_json::to_vec_pretty(&zero_sender_artifact).unwrap(),
    )
    .unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", zero_sender_artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("uses forbidden sender"), "{stdout}");
});

forgetest_init!(symbolic_artifact_replay_accepts_created_sequence_target, |prj, cmd| {
    prj.add_test(
        "SymbolicArtifactCreatedTarget.t.sol",
        r#"
import "forge-std/Test.sol";

contract CreatedTarget {
    SymbolicArtifactCreatedTarget invariantTest;

    constructor(SymbolicArtifactCreatedTarget _invariantTest) {
        invariantTest = _invariantTest;
    }

    function step() external {
        invariantTest.trip();
    }
}

contract Spawner {
    SymbolicArtifactCreatedTarget invariantTest;

    constructor(SymbolicArtifactCreatedTarget _invariantTest) {
        invariantTest = _invariantTest;
    }

    function step() external {
        new CreatedTarget(invariantTest);
    }
}

contract SymbolicArtifactCreatedTarget is Test {
    bool tripped;

    function setUp() public {
        new Spawner(this);
    }

    function trip() external {
        tripped = true;
    }

    function invariant_notTripped() public view {
        assert(!tripped);
    }
}
"#,
    );

    let artifact_path = prj.root().join("created-sequence-target-artifact.json");
    let artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "sequence",
        "test": {
            "contract": "test/SymbolicArtifactCreatedTarget.t.sol:SymbolicArtifactCreatedTarget",
            "test": "invariant_notTripped()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": null
        },
        "replay_semantics": {
            "fail_on_revert": false
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 2,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [
            {
                "warp": null,
                "roll": null,
                "sender": "0x0000000000000000000000000000000000000b0b",
                "target": "0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f",
                "calldata": "0xe25fe175",
                "value": null,
                "contract_name": "Spawner",
                "function_name": "step",
                "signature": "step()",
                "args": "",
                "raw_args": ""
            },
            {
                "warp": null,
                "roll": null,
                "sender": "0x0000000000000000000000000000000000000b0b",
                "target": "0x104fBc016F4bb334D775a19E8A6510109AC63E00",
                "calldata": "0xe25fe175",
                "value": null,
                "contract_name": "CreatedTarget",
                "function_name": "step",
                "signature": "step()",
                "args": "",
                "raw_args": ""
            }
        ]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert!(stdout.contains("panic: assertion failed"), "{stdout}");
    assert!(!stdout.contains("targets unknown function"), "{stdout}");
});

forgetest_init!(symbolic_artifact_replay_ignores_non_target_network_passes, |prj, cmd| {
    prj.add_test(
        "SymbolicArtifactNetworkReplay.t.sol",
        r#"
import "forge-std/Test.sol";

contract SymbolicArtifactNetworkReplay is Test {
    function setUp() public {
        targetContract(address(this));
    }

    function step() external {}

    function invariant_noop() public pure {}

    /// forge-config: default.networks.network = "tempo"
    function test_tempo_marker() public pure {}
}
"#,
    );

    let artifact_path = prj.root().join("network-replay-artifact.json");
    let artifact = serde_json::json!({
        "schema_version": 1,
        "schema": "foundry:symbolic.counterexample@v1",
        "kind": "sequence",
        "test": {
            "contract": "test/SymbolicArtifactNetworkReplay.t.sol:SymbolicArtifactNetworkReplay",
            "test": "invariant_noop()"
        },
        "replay": {
            "required": true,
            "status": "confirmed",
            "reason": null
        },
        "replay_semantics": {
            "fail_on_revert": false
        },
        "bounds": {
            "timeout_seconds": null,
            "loop_bound": null,
            "max_depth": 0,
            "max_paths": 0,
            "invariant_depth": 1,
            "exploration_order": "bfs",
            "max_solver_queries": 0,
            "default_dynamic_length": 0,
            "max_dynamic_length": 0,
            "array_lengths": [],
            "dynamic_lengths": {},
            "default_array_lengths": [],
            "default_bytes_lengths": [],
            "max_calldata_bytes": 0,
            "symbolic_call_targets": false,
            "storage_layout": "solidity"
        },
        "solver": {
            "name": "manual",
            "command": null,
            "portfolio": [],
            "stats": {
                "paths": 0,
                "solver_queries": 0,
                "smt_queries": 0,
                "sat_queries": 0,
                "model_queries": 0,
                "sat_cache_hits": 0,
                "model_cache_hits": 0,
                "heuristic_witnesses": 0,
                "solver_time_ms": 0,
                "smt_input_bytes": 0,
                "smt_max_query_bytes": 0,
                "smt_build_time_ms": 0,
                "smt_max_query_time_ms": 0
            }
        },
        "assumptions": [],
        "call_trace": {
            "available": false,
            "source": null,
            "format": null
        },
        "calls": [{
            "warp": null,
            "roll": null,
            "sender": "0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38",
            "target": "0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
            "calldata": "0xe25fe175",
            "value": null,
            "contract_name": "SymbolicArtifactNetworkReplay",
            "function_name": "step",
            "signature": "step()",
            "args": "",
            "raw_args": ""
        }]
    });
    std::fs::write(&artifact_path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();

    let stdout = cmd
        .forge_fuse()
        .args(["test", "--replay-symbolic-artifact", artifact_path.to_str().unwrap()])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(stdout.contains("[PASS] invariant_noop()"), "{stdout}");
    assert!(!stdout.contains("was not found"), "{stdout}");
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
