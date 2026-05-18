use super::symbolic_helpers::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// devdacian/solidity-fuzzing-comparison — "Rarely False" challenge.
// ---------------------------------------------------------------------------
// Only Halmos & Certora solved this; Echidna, Medusa, and Foundry-fuzz all
// failed. A perfect minimal regression for the symbolic engine.
forgetest_init!(devdacian_rarely_false_parity, |prj, cmd| {
    skip_unless_z3!("devdacian_rarely_false_parity");

    prj.add_test(
        "DevdacianRarelyFalse.t.sol",
        r#"
contract DevdacianRarelyFalse {
    // The bug is only triggered by an exact uint256 with two specific 32-bit
    // halves and a specific low byte — fuzzers basically never find it but the
    // SMT solver can.
    function checkRarelyFalse(uint256 x) public pure {
        uint256 hi = x >> 192;
        uint256 mid = (x >> 96) & ((1 << 96) - 1);
        uint256 lo = x & 0xff;

        if (hi == 0x1234 && mid == 0xCAFE && lo == 0x42) {
            assert(false);
        }
    }
}
"#,
    );

    // Three bit-field constraints uniquely identify x → deterministic witness.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkRarelyFalse"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/DevdacianRarelyFalse.t.sol:DevdacianRarelyFalse
[FAIL: panic: assertion failed (0x01); counterexample: calldata=0x1d03c04b000000000000123400000000000000000000cafe000000000000000000000042 args=[29251294086901932359474778716264896192253236938588505753256002 [2.925e61]]] checkRarelyFalse(uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
