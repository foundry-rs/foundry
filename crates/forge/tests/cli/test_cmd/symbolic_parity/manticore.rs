use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// Manticore-style multi-transaction state exploration: the bug requires a
// setup transaction before the final assertion can fail.
forgetest_init!(manticore_multitx_state_machine, |prj, cmd| {
    skip_unless_z3!("manticore_multitx_state_machine");

    prj.add_test(
        "ManticoreMultiTx.t.sol",
        r#"
import "forge-std/Test.sol";

contract ArmTarget {
    bool public armed;

    function arm(uint256 key) public {
        if (key == 0xfeed) armed = true;
    }
}

contract ManticoreMultiTx is Test {
    ArmTarget target;

    function setUp() public {
        target = new ArmTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_neverArmed() public view {
        assert(!target.armed());
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_neverArmed",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/ManticoreMultiTx.t.sol:ManticoreMultiTx
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 1, shrunk: 1)
		[SENDER] addr=[test/ManticoreMultiTx.t.sol:ArmTarget]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=arm(uint256) [ARGS]
 invariant_neverArmed() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
