use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

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
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/SwcUncheckedCall.t.sol:SwcUncheckedCallTarget
[FAIL: incomplete symbolic execution (Stuck): unsupported symbolic execution feature: symbolic CALL target] checkUncheckedCall(address) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
