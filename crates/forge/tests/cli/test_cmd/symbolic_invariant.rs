use super::symbolic_helpers::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// EIP-1153 transient storage is per-transaction scratch space. The symbolic
// invariant runner must clear `state.world.transient_storage` at the boundary
// of every top-level sequence step. The target below has two entry points:
// `poke` writes a symbolic sentinel into transient slot 0, and `peek` reverts
// if transient slot 0 is non-zero. Because each call is a fresh top-level
// transaction, `peek` must always observe zero — regardless of how many
// `poke(sentinel)` calls preceded it.
forgetest_init!(symbolic_transient_storage_clears_between_sequence_steps, |prj, cmd| {
    skip_unless_z3!("symbolic_transient_storage_clears_between_sequence_steps");

    prj.add_test(
        "SymbolicTransientStorageInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract TransientTarget {
    function poke(uint256 sentinel) external {
        assembly { tstore(0, sentinel) }
    }

    function peek() external view {
        uint256 v;
        assembly { v := tload(0) }
        // Must hold at the start of every top-level call: transient storage
        // from a prior step must have been cleared.
        require(v == 0, "transient storage leaked across top-level steps");
    }
}

contract SymbolicTransientStorageInvariant is Test {
    TransientTarget target;

    function setUp() public {
        target = new TransientTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_transientClearsBetweenSteps() public view {
        target.peek();
    }
}
"#,
    );

    assert_symbolic(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_transientClearsBetweenSteps",
    ]))
    .success()
    .stdout_eq(str![[r#"
...
Ran 1 test for test/SymbolicTransientStorageInvariant.t.sol:SymbolicTransientStorageInvariant
[PASS] invariant_transientClearsBetweenSteps() ([METRICS])
...
"#]]);
});
