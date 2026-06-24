use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// Echidna flags.sol — canonical multi-flag puzzle.
// ---------------------------------------------------------------------------
// Source: https://github.com/crytic/echidna/blob/master/tests/solidity/basic/flags.sol
// Echidna finds a sequence that falsifies `echidna_sometimesfalse`.
// We port it as a stateful symbolic invariant with bounded depth.
forgetest_init!(echidna_flags_parity, |prj, cmd| {
    skip_unless_z3!("echidna_flags_parity");

    prj.add_test(
        "EchidnaFlagsParity.t.sol",
        r#"
import "forge-std/Test.sol";

contract EchidnaFlagsTarget {
    // CI-bounded variant of crytic/echidna's `tests/solidity/basic/flags.sol`.
    // Uses uint8 inputs so the solver does not have to reason about full int256
    // modulo branching; the structure of the puzzle (must call set0 first to
    // open set1's branch) is preserved.
    bool public flag0 = true;
    bool public flag1 = true;

    function set0(uint8 val) public {
        if (val == 0) flag0 = false;
    }

    function set1(uint8 val) public {
        if (val == 0 && !flag0) flag1 = false;
    }
}

contract EchidnaFlagsParity is Test {
    EchidnaFlagsTarget target;

    function setUp() public {
        target = new EchidnaFlagsTarget();
        targetContract(address(target));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_flag1_holds() public view {
        assertTrue(target.flag1());
    }
}
"#,
    );

    // Witness args are free uint8 → use `[ARGS]` redaction.
    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_flag1_holds",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/EchidnaFlagsParity.t.sol:EchidnaFlagsParity
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 2, shrunk: 2)
		[SENDER] addr=[test/EchidnaFlagsParity.t.sol:EchidnaFlagsTarget]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=set0(uint8) [ARGS]
		[SENDER] addr=[test/EchidnaFlagsParity.t.sol:EchidnaFlagsTarget]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=set1(uint8) [ARGS]
 invariant_flag1_holds() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});

// ---------------------------------------------------------------------------
// Echidna overflow mode — Solidity 0.8 over/underflow detection.
// ---------------------------------------------------------------------------
// Mirrors Echidna's `--test-mode overflow` on a buggy add.
forgetest_init!(echidna_overflow_unchecked_add, |prj, cmd| {
    skip_unless_z3!("echidna_overflow_unchecked_add");

    prj.add_test(
        "EchidnaOverflowParity.t.sol",
        r#"
contract EchidnaOverflowParity {
    // Buggy: uses unchecked so overflow silently wraps; assertion catches it.
    function checkNoOverflow(uint256 a, uint256 b) public pure {
        unchecked {
            uint256 sum = a + b;
            // This holds only if a + b doesn't overflow.
            assert(sum >= a);
        }
    }
}
"#,
    );

    // Witness values (a, b) are free uint256 — Z3 picks any pair where a+b
    // overflows. Redact `calldata=` and `args=[...]` via
    // [`assert_symbolic_witness`] so the snapshot captures the shape but not
    // the solver's arbitrary choice.
    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkNoOverflow"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/EchidnaOverflowParity.t.sol:EchidnaOverflowParity
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkNoOverflow(uint256,uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
