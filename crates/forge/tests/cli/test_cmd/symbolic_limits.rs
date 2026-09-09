use super::symbolic_helpers::assert_relevant_lines;
use foundry_common::sh_eprintln;
use foundry_test_utils::{forgetest_init, util::OutputExt};
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

forgetest_init!(symbolic_limits_riddle_counterexample_replays, |prj, cmd| {
    if should_skip("symbolic_limits_riddle_counterexample_replays") {
        return;
    }

    prj.add_test(
        "SymbolicLimitsRiddle.t.sol",
        r#"
contract SymbolicLimitsRiddle {
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
        .args([
            "test",
            "--symbolic",
            "--symbolic-timeout",
            "300",
            "--symbolic-width",
            "512",
            "--symbolic-depth",
            "50000",
            "--symbolic-max-solver-queries",
            "20000",
            "--match-test",
            "check_riddle",
        ])
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
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
(paths:
"#]],
    );
    assert!(!stdout.contains("symbolic counterexample did not replay"), "{stdout}");
    assert!(!stdout.contains("incomplete symbolic execution"), "{stdout}");
});

forgetest_init!(symbolic_limits_reports_path_width_exhaustion, |prj, cmd| {
    if should_skip("symbolic_limits_reports_path_width_exhaustion") {
        return;
    }

    prj.add_test(
        "SymbolicLimitsPathWidth.t.sol",
        r#"
contract SymbolicLimitsPathWidth {
    function checkWidth(uint8 x) public pure {
        uint256 acc;

        if ((x & 0x01) != 0) acc += 1; else acc += 2;
        if ((x & 0x02) != 0) acc += 4; else acc += 8;
        if ((x & 0x04) != 0) acc += 16; else acc += 32;
        if ((x & 0x08) != 0) acc += 64; else acc += 128;
        if ((x & 0x10) != 0) acc += 256; else acc += 512;

        assert(acc != 0);
    }
}
"#,
    );

    let stdout = cmd
        .args(["test", "--symbolic", "--symbolic-width", "2", "--match-test", "checkWidth"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkWidth(uint8)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic path limit exceeded (2)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck)
"#]],
    );
});

forgetest_init!(symbolic_limits_reports_execution_depth_exhaustion, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--symbolic-depth", "8", "--match-test", "checkDepth"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkDepth(uint256)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic depth limit exceeded (8)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck)
"#]],
    );
});

forgetest_init!(symbolic_limits_reports_calldata_budget_exhaustion, |prj, cmd| {
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

    let stdout = cmd
        .args(["test", "--symbolic", "--match-test", "checkCalldataBudget"])
        .assert_failure()
        .get_output()
        .stdout_lossy();

    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
checkCalldataBudget(bytes)
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
symbolic calldata size exceeds configured max
"#]],
    );
    assert_relevant_lines(
        &stdout,
        foundry_test_utils::str![[r#"
incomplete symbolic execution (Stuck)
"#]],
    );
});

forgetest_init!(symbolic_limits_invariant_depth_changes_result, |prj, _cmd| {
    if should_skip("symbolic_limits_invariant_depth_changes_result") {
        return;
    }

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

    let passing = prj
        .forge_command()
        .args([
            "test",
            "--symbolic",
            "--symbolic-invariant-depth",
            "1",
            "--match-test",
            "invariant_valueNeverTwo",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &passing,
        foundry_test_utils::str![[r#"
[PASS] invariant_valueNeverTwo()
"#]],
    );

    let failing = prj
        .forge_command()
        .args([
            "test",
            "--symbolic",
            "--symbolic-invariant-depth",
            "2",
            "--match-test",
            "invariant_valueNeverTwo",
        ])
        .assert_failure()
        .get_output()
        .stdout_lossy();
    assert_relevant_lines(
        &failing,
        foundry_test_utils::str![[r#"
[FAIL:
"#]],
    );
    assert_relevant_lines(
        &failing,
        foundry_test_utils::str![[r#"
symbolic invariant counterexample
"#]],
    );
    assert_relevant_lines(
        &failing,
        foundry_test_utils::str![[r#"
invariant_valueNeverTwo()
"#]],
    );
});
