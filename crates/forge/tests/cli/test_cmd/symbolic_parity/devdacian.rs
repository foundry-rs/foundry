use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// devdacian/solidity-fuzzing-comparison — "Rarely False" challenge (#6).
// ---------------------------------------------------------------------------
// Only Halmos & Certora solved this upstream; Echidna, Medusa, and
// Foundry-fuzz all failed. A perfect minimal regression for the symbolic
// engine on modular / divisibility reasoning (`bv-urem` over bounded `n`).
forgetest_init!(devdacian_rarely_false_parity, |prj, cmd| {
    skip_unless_z3!("devdacian_rarely_false_parity");

    prj.add_test(
        "DevdacianRarelyFalse.t.sol",
        r#"
import "forge-std/Test.sol";

contract DevdacianRarelyFalse is Test {
    uint256 constant private OFFSET = 1234;
    uint256 constant private POW    = 80;

    function checkRarelyFalse(uint256 n) public pure {
        // Match upstream input precondition: n in [1, type(uint256).max - OFFSET].
        vm.assume(n >= 1);
        vm.assume(n <= type(uint256).max - OFFSET);

        // Upstream assertion: `t(_rarelyFalse(n + OFFSET, POW), ...)`
        // where `_rarelyFalse(x, e)` returns `false` iff `x % 2**e == 0`.
        // So the assertion should hold unless `(n + OFFSET) % 2**POW == 0`.
        assert((n + OFFSET) % (1 << POW) != 0);
    }
}
"#,
    );

    // Modular constraint with non-power-of-two offset uniquely solvable by SMT
    // (bv-urem over bounded n); fuzzers cannot stumble on it.
    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkRarelyFalse"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/DevdacianRarelyFalse.t.sol:DevdacianRarelyFalse
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkRarelyFalse(uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
