use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// Scribble/Harvey-style instrumented property: the annotation is represented
// as an inserted assert, which is what the engine ultimately has to prove.
forgetest_init!(scribble_instrumented_erc20_supply_property, |prj, cmd| {
    skip_unless_z3!("scribble_instrumented_erc20_supply_property");

    prj.add_test(
        "ScribbleInstrumentedSupply.t.sol",
        r#"
contract ScribbleInstrumentedSupply {
    mapping(address => uint256) balanceOf;
    uint256 totalSupply;

    function mint(address to, uint256 amount) public {
        require(to != address(0));
        balanceOf[to] += amount;
        totalSupply += amount + 1;
    }

    function checkSupplyAnnotation(address to, uint8 amount) public {
        mint(to, amount);
        assert(totalSupply == balanceOf[to]);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkSupplyAnnotation",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/ScribbleInstrumentedSupply.t.sol:ScribbleInstrumentedSupply
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkSupplyAnnotation(address,uint8) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
