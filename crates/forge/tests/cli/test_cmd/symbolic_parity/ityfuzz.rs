use super::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// ItyFuzz paper "SimpleState" — narrow-state walk fuzzers struggle with.
// ---------------------------------------------------------------------------
// Already partially covered in symbolic_conformance.rs as a passing case.
// Here we add the version with the BUG (final phase reachable via specific
// sequence of magic numbers) and assert the symbolic engine catches it.
forgetest_init!(ityfuzz_simple_state_buggy, |prj, cmd| {
    skip_unless_z3!("ityfuzz_simple_state_buggy");

    prj.add_test(
        "IfFuzzSimpleStateBuggy.t.sol",
        r#"
import "forge-std/Test.sol";

contract SimpleStateMachine {
    uint256 public phase;

    function step1(uint256 v) external {
        if (v == 1337) phase = 1;
    }

    function step2(uint256 v) external {
        if (phase == 1 && v == 7331) phase = 2;
    }

    function step3(uint256 v) external {
        if (phase == 2 && v == 12345) phase = 3;
    }
}

contract IfFuzzSimpleStateBuggy is Test {
    SimpleStateMachine sm;

    function setUp() public {
        sm = new SimpleStateMachine();
        targetContract(address(sm));
    }

    /// forge-config: default.symbolic.invariant_depth = 3
    function invariant_phaseUnderThree() public view {
        assertLt(sm.phase(), 3);
    }
}
"#,
    );

    // Magic numbers (1337, 7331, 12345) are forced by the branch structure.
    // Symbolic senders are masked via the [SENDER] redaction in
    // `assert_symbolic`.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_phaseUnderThree"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/IfFuzzSimpleStateBuggy.t.sol:IfFuzzSimpleStateBuggy
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 3, shrunk: 3)
		[SENDER] addr=[test/IfFuzzSimpleStateBuggy.t.sol:SimpleStateMachine]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step1(uint256) args=[1337]
		[SENDER] addr=[test/IfFuzzSimpleStateBuggy.t.sol:SimpleStateMachine]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step2(uint256) args=[7331]
		[SENDER] addr=[test/IfFuzzSimpleStateBuggy.t.sol:SimpleStateMachine]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=step3(uint256) args=[12345 [1.234e4]]
 invariant_phaseUnderThree() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
