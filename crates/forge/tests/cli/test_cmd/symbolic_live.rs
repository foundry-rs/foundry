use super::symbolic_helpers::json_test_result;
use crate::skip_unless_z3;
use foundry_config::InvariantDepthMode;
use foundry_test_utils::forgetest_init;

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

#[forgetest_init]
fn symbolic_live_zero_query_budget(prj: _, cmd: _) {
    prj.add_test("LiveTwoStep.t.sol", TWO_STEP);
    prj.update_config(|config| {
        config.invariant.runs = 100;
        config.invariant.depth = 2;
        config.invariant.dictionary.dictionary_weight = 0;
        config.symbolic.live = true;
        config.symbolic.max_solver_queries = 0;
    });
    let output = cmd
        .args(["test", "--json", "--threads", "1", "--fuzz-seed", "0x1234"])
        .assert_success()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_a()");
    assert_eq!(result["status"], "Success");
    assert_eq!(result["symbolic_live"]["solver_queries"], 0);
    assert_eq!(result["symbolic_live"]["completed"], 0);
    assert_eq!(result["symbolic_live"]["scheduled"], 0);
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
    for (frontiers, queries, timeout) in [(0, 100, 10), (100, 100, 0), (1, 1, 10)] {
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
        if frontiers == 0 || timeout == 0 {
            assert_eq!(stats["completed"], 0);
            assert_eq!(stats["scheduled"], 0);
        }
    }
}
