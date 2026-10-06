use super::symbolic_helpers::assert_symbolic_witness;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, str};
use std::{env, process::Command};

fn symbolic_limits_enabled() -> bool {
    env::var_os("SYMBOLIC_LIMITS").is_some()
}

fn z3_available() -> bool {
    Command::new("z3").arg("--version").output().is_ok_and(|output| output.status.success())
}

fn should_skip(test: &str) -> bool {
    if !symbolic_limits_enabled() {
        let _ = sh_eprintln!("skipping {test} because SYMBOLIC_LIMITS is not set");
        return true;
    }
    if !z3_available() {
        let _ = sh_eprintln!("skipping {test} because z3 is not available");
        return true;
    }
    false
}

#[forgetest_init]
fn symbolic_limits_reports_execution_depth_exhaustion(prj: _, cmd: _) {
    if should_skip("symbolic_limits_reports_execution_depth_exhaustion") {
        return;
    }

    prj.add_test(
        "SymbolicLimitsDepth.t.sol",
        r#"
contract SymbolicLimitsDepth {
    function checkDepth(uint256 x) public pure {
        uint256 y = x;
        y += 1;
        y += 2;
        y += 3;
        y += 4;
        y += 5;
        y += 6;
        y += 7;
        y += 8;
        assert(y != type(uint256).max);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--symbolic-depth",
        "8",
        "--match-test",
        "checkDepth",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicLimitsDepth.t.sol:SymbolicLimitsDepth
[FAIL: incomplete symbolic execution (Stuck): symbolic depth limit exceeded (8)] checkDepth(uint256) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_limits_reports_calldata_budget_exhaustion(prj: _, cmd: _) {
    if should_skip("symbolic_limits_reports_calldata_budget_exhaustion") {
        return;
    }

    prj.add_test(
        "SymbolicLimitsCalldataBudget.t.sol",
        r#"
contract SymbolicLimitsCalldataBudget {
    /// forge-config: default.symbolic.array_lengths = [64]
    /// forge-config: default.symbolic.max_calldata_bytes = 96
    function checkCalldataBudget(bytes memory data) public pure {
        assert(data.length == 64);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkCalldataBudget",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicLimitsCalldataBudget.t.sol:SymbolicLimitsCalldataBudget
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic calldata size exceeds configured max] checkCalldataBudget(bytes) ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}

#[forgetest_init]
fn symbolic_limits_invariant_depth_changes_result(prj: _) {
    if should_skip("symbolic_limits_invariant_depth_changes_result") {
        return;
    }

    // `--symbolic` still runs the concrete invariant campaign, so keep it at depth 1 to let the
    // symbolic depth decide whether the two-call sequence is found.
    prj.update_config(|config| config.invariant.depth = 1);
    prj.add_test(
        "SymbolicLimitsInvariantDepth.t.sol",
        r#"
import "forge-std/Test.sol";

contract LimitsCounter {
    uint256 public value;

    function inc() external {
        value++;
    }
}

contract SymbolicLimitsInvariantDepth is Test {
    LimitsCounter counter;

    function setUp() public {
        counter = new LimitsCounter();
        targetContract(address(counter));
    }

    function invariant_valueNeverTwo() public view {
        assertTrue(counter.value() != 2);
    }
}
"#,
    );

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--symbolic-invariant-depth",
        "1",
        "--match-test",
        "invariant_valueNeverTwo",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicLimitsInvariantDepth.t.sol:SymbolicLimitsInvariantDepth
[PASS] invariant_valueNeverTwo() ([METRICS])

╭---------------+----------+-------+---------+----------╮
| Contract      | Selector | Calls | Reverts | Discards |
+=======================================================+
| LimitsCounter | inc      | 256   | 0       | 0        |
╰---------------+----------+-------+---------+----------╯

Suite result: ok. 1 passed; 0 failed; 0 skipped; [ELAPSED]
...
"#]]);

    assert_symbolic_witness(prj.forge_command().args([
        "test",
        "--symbolic",
        "--symbolic-invariant-depth",
        "2",
        "--match-test",
        "invariant_valueNeverTwo",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicLimitsInvariantDepth.t.sol:SymbolicLimitsInvariantDepth
[FAIL: assertion failed]
	[Sequence] (original: 2, shrunk: 2)
		[SENDER] [SENDER] calldata=inc() [ARGS]
		[SENDER] [SENDER] calldata=inc() [ARGS]
 invariant_valueNeverTwo() ([METRICS])
Suite result: FAILED. 0 passed; 1 failed; 0 skipped; [ELAPSED]
...
"#]]);
}
