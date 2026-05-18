use super::symbolic_helpers::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::forgetest_init;

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
    .failure();
});
