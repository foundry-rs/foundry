use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// hevm-style symbolic calldata constraint: a magic calldata value should be
// solved directly rather than found through random fuzzing.
forgetest_init!(hevm_symbolic_calldata_constraint, |prj, cmd| {
    skip_unless_z3!("hevm_symbolic_calldata_constraint");

    prj.add_test(
        "HevmCalldataConstraint.t.sol",
        r#"
contract HevmCalldataConstraint {
    function checkMagic(bytes4 selector, uint256 x) public pure {
        require(selector == 0xdeadbeef);
        require(x == 0x1234);
        assert(false);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkMagic"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/HevmCalldataConstraint.t.sol:HevmCalldataConstraint
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkMagic(bytes4,uint256) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
