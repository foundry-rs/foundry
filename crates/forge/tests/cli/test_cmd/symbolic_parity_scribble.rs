use super::symbolic_helpers::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::forgetest_init;

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
    .failure();
});
