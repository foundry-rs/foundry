use super::symbolic_helpers::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::forgetest_init;

// SWC-104 unchecked low-level call: failure is reachable when the callee
// returns false, but the caller still marks the operation as complete.
forgetest_init!(swc_unchecked_low_level_call, |prj, cmd| {
    skip_unless_z3!("swc_unchecked_low_level_call");

    prj.add_test(
        "SwcUncheckedCall.t.sol",
        r#"
contract SwcUncheckedCallTarget {
    bool ok;

    function run(address target) public {
        target.call(abi.encodeWithSignature("missing()"));
        ok = true;
    }

    function checkUncheckedCall(address target) public {
        run(target);
        assert(!ok);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkUncheckedCall"]))
        .failure();
});
