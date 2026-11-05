use super::symbolic_helpers::json_test_result;
use crate::skip_unless_z3;
use foundry_config::InvariantDepthMode;
use foundry_test_utils::forgetest_init;

#[cfg(unix)]
use std::{
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{Duration, Instant},
};

// Two wide arithmetic gates keep this regression outside ordinary random input coverage.
// The second gate is only visible after replaying a candidate for the first gate.
const TWO_STEP: &str = r#"
import "forge-std/Test.sol";

contract LiveTarget {
    uint256 public stage;

    function step(uint256 x, uint256 y) external {
        unchecked {
            if (stage == 0 && x * 3 + y * 5 == 0xdeadbeef123456789abcdef01234567890) {
                stage = 1;
            } else if (stage == 1 && x * 3 + y * 5 == 0xabcdef019876543210fedcba9876543210) {
                stage = 2;
            }
        }
    }
}

contract LiveTwoStep is Test {
    LiveTarget target;

    function setUp() public {
        target = new LiveTarget();
        targetContract(address(target));
        targetSender(address(this));
    }

    function invariant_a() public pure { }

    function invariant_z() public view {
        assertLt(target.stage(), 2);
    }
}
"#;

#[forgetest_init]
fn symbolic_live_two_step(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_live_two_step");
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    prj.update_config(|config| {
        config.invariant.runs = 1_000;
        config.invariant.depth = 2;
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.timeout = Some(10);
    });
    let output = cmd
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert!(result.get("symbolic_live").is_none());

    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(10);
    });
    let output = cmd
        .forge_fuse()
        .args(["test", "--symbolic-live", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["status"], "Failure", "{result}");
    assert!(result["symbolic"].is_null());
    assert!(result["symbolic_live"]["scheduled"].as_u64().unwrap() >= 2, "{result}");
    let failures = result["invariant_failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{result}");
    assert_eq!(failures[0]["name"], "invariant_z");
    assert_eq!(failures[0]["counterexample"]["Sequence"][1].as_array().unwrap().len(), 2);
    // Persisted failures must replay through the ordinary invariant path without the worker.
    prj.update_config(|config| config.invariant.runs = 0);
    let output = cmd
        .forge_fuse()
        .args(["test", "--json", "--threads", "1"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let replayed = json_test_result(&output, "invariant_a()");
    assert!(replayed.get("symbolic_live").is_none());
    assert_eq!(replayed["invariant_failures"][0]["name"], "invariant_z");
}

// Every call carries a warp and a roll once delays are configured; the worker applies them
// concretely instead of rejecting the frontier.
#[forgetest_init]
fn symbolic_live_solves_delayed_calls(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_live_solves_delayed_calls");
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(10);
        config.invariant.depth = 2;
        config.invariant.max_time_delay = Some(3600);
        config.invariant.max_block_delay = Some(300);
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.live = true;
        config.symbolic.timeout = Some(10);
    });
    let output = cmd
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["invariant_failures"][0]["name"], "invariant_z", "{result}");
    assert!(result["symbolic_live"]["scheduled"].as_u64().unwrap() >= 2, "{result}");
    assert!(result["symbolic_live"]["solver_queries"].as_u64().unwrap() > 0, "{result}");
}

#[forgetest_init]
fn symbolic_live_parallel_random_depth(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_live_parallel_random_depth");
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(10);
        config.invariant.depth = 2;
        config.invariant.min_depth = 1;
        config.invariant.depth_mode = InvariantDepthMode::Random;
        config.invariant.check_interval = 2;
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.live = true;
        config.symbolic.timeout = Some(10);
    });
    let output = cmd
        .args(["test", "--json", "--threads", "2", "--fuzz-seed", "0x1234"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["invariant_failures"][0]["name"], "invariant_z");
    assert_eq!(
        result["invariant_failures"][0]["counterexample"]["Sequence"][1].as_array().unwrap().len(),
        2
    );
    assert!(result["symbolic_live"]["scheduled"].as_u64().unwrap() >= 2);
}

#[forgetest_init]
fn symbolic_live_frontier_and_time_budgets(prj: _, cmd: _) {
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    for (frontiers, queries, timeout) in [(0, 100, 10), (100, 100, 0), (100, 0, 10), (1, 1, 10)] {
        prj.update_config(|config| {
            config.invariant.runs = 100;
            config.invariant.depth = 2;
            config.invariant.dictionary.dictionary_weight = 0;
            config.symbolic.frontier_limit = frontiers;
            config.symbolic.max_solver_queries = queries;
            config.symbolic.timeout = Some(timeout);
        });
        cmd.forge_fuse().env("FOUNDRY_SYMBOLIC_LIVE", "true");
        let output = cmd
            .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
            .assert_success()
            .get_output()
            .stdout
            .clone();
        let result = json_test_result(&output, "invariant_a()");
        let stats = &result["symbolic_live"];
        assert!(stats["submitted"].as_u64().unwrap() <= frontiers as u64);
        assert!(stats["solver_queries"].as_u64().unwrap() <= queries as u64);
        if frontiers == 0 || queries == 0 || timeout == 0 {
            assert_eq!(stats["completed"], 0);
            assert_eq!(stats["scheduled"], 0);
        }
    }
}

#[cfg(unix)]
#[forgetest_init]
fn symbolic_live_bounds_solver_version_probe(prj: _, cmd: _) {
    let solver = prj.root().join("slow-version-solver");
    std::fs::write(
        &solver,
        r#"#!/bin/sh
echo $$ > "$0.pid"
exec sleep 10
"#,
    )
    .unwrap();
    std::fs::set_permissions(&solver, std::fs::Permissions::from_mode(0o755)).unwrap();
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(2);
        config.invariant.depth = 2;
        config.symbolic.live = true;
        config.symbolic.timeout = Some(2);
        config.symbolic.frontier_limit = 1;
        config.symbolic.solver_command = Some(solver.display().to_string());
    });
    // Keep compiler installation and compilation outside the shutdown measurement.
    cmd.arg("build").assert_success();
    let started = Instant::now();
    let output = cmd
        .forge_fuse()
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    assert!(started.elapsed() < Duration::from_secs(5), "worker shutdown exceeded its budget");
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["symbolic_live"]["completed"], 1);
    assert_eq!(result["symbolic_live"]["scheduled"], 0);
    let pid = std::fs::read_to_string(solver.with_extension("pid")).unwrap();
    assert!(
        !Command::new("kill").args(["-0", pid.trim()]).output().unwrap().status.success(),
        "solver version probe survived campaign shutdown"
    );
}

#[forgetest_init]
fn symbolic_live_one_second_budget(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_live_one_second_budget");
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(3);
        config.invariant.depth = 2;
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.live = true;
        config.symbolic.timeout = Some(1);
    });
    let output = cmd
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert!(result["symbolic_live"]["solver_queries"].as_u64().unwrap() > 0, "{result}");
}

// Many handlers add dispatcher comparisons whose other side fuzzing already reaches. They must
// not crowd the gates out of the worker's backlog.
#[forgetest_init]
fn symbolic_live_wide_target(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_live_wide_target");
    let handlers = (0..24)
        .map(|i| {
            format!(
                "    function f{i}(uint256 a) external {{ if (a % {} == {i}) counter += 1; }}\n",
                i + 7
            )
        })
        .collect::<String>();
    let source = TWO_STEP.replacen(
        "    uint256 public stage;\n",
        &format!("    uint256 public stage;\n    uint256 public counter;\n{handlers}"),
        1,
    );
    prj.add_test("LiveWide.t.sol", &source);
    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(10);
        config.invariant.depth = 100;
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.live = true;
        config.symbolic.timeout = Some(10);
    });
    let output = cmd
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["invariant_failures"][0]["name"], "invariant_z", "{result}");
    assert!(result["symbolic_live"]["scheduled"].as_u64().unwrap() >= 2, "{result}");
}

// One equality guards every stage through a stored secret, so each later attempt must run from
// the state left by the previous solve even though both sides of the site were already reached.
// Random depths end some runs right after a solve, so resumed runs must also retry the gate.
#[forgetest_init]
fn symbolic_live_retries_shared_gate(prj: _, cmd: _) {
    skip_unless_z3!("symbolic_live_retries_shared_gate");
    let source = TWO_STEP
        .replacen(
            "    uint256 public stage;\n",
            "    uint256 public stage;
    uint256[3] secrets = [
        uint256(0xdeadbeef123456789abcdef01234567890),
        0xabcdef019876543210fedcba9876543210,
        0x1234567890abcdef1234567890abcdef12
    ];
",
            1,
        )
        .replacen(
            "            if (stage == 0 && x * 3 + y * 5 == 0xdeadbeef123456789abcdef01234567890) {
                stage = 1;
            } else if (stage == 1 && x * 3 + y * 5 == 0xabcdef019876543210fedcba9876543210) {
                stage = 2;
            }
",
            "            if (stage < 3 && x * 3 + y * 5 == secrets[stage]) stage++;\n",
            1,
        )
        .replacen("assertLt(target.stage(), 2)", "assertLt(target.stage(), 3)", 1);
    assert!(source.contains("stage++") && source.contains("stage(), 3"));
    prj.add_test("LiveSharedGate.t.sol", &source);
    prj.update_config(|config| {
        config.invariant.runs = u32::MAX;
        config.invariant.timeout = Some(10);
        config.invariant.depth = 3;
        config.invariant.min_depth = 1;
        config.invariant.depth_mode = InvariantDepthMode::Random;
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.live = true;
        config.symbolic.timeout = Some(10);
    });
    let output = cmd
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["invariant_failures"][0]["name"], "invariant_z", "{result}");
    assert_eq!(
        result["invariant_failures"][0]["counterexample"]["Sequence"][1].as_array().unwrap().len(),
        3,
        "{result}"
    );
}
