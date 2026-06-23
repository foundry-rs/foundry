use super::symbolic_helpers::{assert_symbolic, assert_symbolic_witness};
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

// A target function that branches symbolically into a revert path and a
// state-mutating path. With `fail_on_revert = false` and `invariant_depth = 2`,
// the engine must continue exploring non-reverting symbolic branches even when
// other branches of the same function revert; otherwise it would silently
// under-approximate and miss the counter increment below.
forgetest_init!(symbolic_revert_branches_do_not_swallow_non_revert_paths, |prj, cmd| {
    skip_unless_z3!("symbolic_revert_branches_do_not_swallow_non_revert_paths");

    prj.add_test(
        "SymbolicRevertBranchInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract RevertBranchTarget {
    uint256 public counter;

    function step(uint8 mode) external {
        if (mode == 0) {
            revert("symbolic revert branch");
        }
        unchecked { counter += 1; }
    }
}

contract SymbolicRevertBranchInvariant is Test {
    RevertBranchTarget target;

    function setUp() public {
        target = new RevertBranchTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    /// forge-config: default.invariant.fail_on_revert = false
    function invariant_counterStaysZero() public view {
        assertEq(target.counter(), 0);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_counterStaysZero",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicRevertBranchInvariant.t.sol:SymbolicRevertBranchInvariant
[FAIL: assertion failed: 1 != 0]
...
 invariant_counterStaysZero() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});

// Top-level invariant sequence calls must look up code through the symbolic
// world overlay so that prior-step `vm.etch` writes are visible. The target
// below deploys a `Counter` whose `value()` returns 0, then exposes an
// `etchCounter` step that overwrites that address with bytecode that returns
// 42 for any call. If the engine fetched code from the backend instead of the
// overlay, the etch effect would be invisible at the next step and the
// (intentionally false) invariant would silently hold.
forgetest_init!(symbolic_invariant_sees_etched_code_via_overlay, |prj, cmd| {
    skip_unless_z3!("symbolic_invariant_sees_etched_code_via_overlay");

    prj.add_test(
        "SymbolicOverlayCodeInvariant.t.sol",
        r#"
import "forge-std/Test.sol";

contract Counter {
    function value() external pure returns (uint256) {
        return 0;
    }
}

contract AlwaysReturns42 {
    fallback() external {
        assembly {
            mstore(0, 42)
            return(0, 32)
        }
    }
}

contract OverlayTarget is Test {
    Counter public c;
    address etched;

    constructor() {
        c = new Counter();
        etched = address(new AlwaysReturns42());
    }

    function etchCounter() external {
        vm.etch(address(c), etched.code);
    }

    function callCounter() external view returns (uint256) {
        return c.value();
    }
}

contract SymbolicOverlayCodeInvariant is Test {
    OverlayTarget t;

    function setUp() public {
        t = new OverlayTarget();
        targetContract(address(t));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_counterAlwaysReturnsZero() public view {
        assertEq(t.callCounter(), 0);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_counterAlwaysReturnsZero",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SymbolicOverlayCodeInvariant.t.sol:SymbolicOverlayCodeInvariant
[FAIL: assertion failed: 42 != 0]
...
 invariant_counterAlwaysReturnsZero() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});
