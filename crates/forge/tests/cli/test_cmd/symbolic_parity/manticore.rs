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
contract ManticoreMultiTx {
    bool armed;

    /// forge-config: default.symbolic.invariant_depth = 2
    function arm(uint256 key) public {
        if (key == 0xfeed) armed = true;
    }

    function invariant_neverArmed() public view {
        assert(!armed);
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
[FAIL: failed to set up invariant testing environment: No contracts to fuzz.] invariant_neverArmed() (runs: 0, calls: 0, reverts: 0)

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

[SEED] (use `--fuzz-seed` to reproduce)

"#]]);
});
