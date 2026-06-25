use super::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// Medusa-style assertion: magic-constant trap on a single value.
// ---------------------------------------------------------------------------
// Source equivalent: crytic/medusa/tests/contracts/assertions/* — assert that
// a specific symbolic input does NOT hit a particular branch. The symbolic
// engine should find the magic value directly via Z3.
forgetest_init!(medusa_assertion_magic_constant, |prj, cmd| {
    skip_unless_z3!("medusa_assertion_magic_constant");

    prj.add_test(
        "MedusaAssertionParity.t.sol",
        r#"
contract MedusaAssertionParity {
    function checkNoMagic(uint256 x) public pure {
        // The magic constant is deep enough that random fuzzing struggles
        // to hit it within a CI budget, but the SMT solver finds it instantly.
        assert(x != 0xDEADBEEFCAFEBABE);
    }
}
"#,
    );

    // Magic constant is uniquely determined → snapshot the full output.
    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkNoMagic"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/MedusaAssertionParity.t.sol:MedusaAssertionParity
[FAIL: panic: assertion failed (0x01); counterexample: calldata=0xda659cbe000000000000000000000000000000000000000000000000deadbeefcafebabe args=[16045690984503098046 [1.604e19]]] checkNoMagic(uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
