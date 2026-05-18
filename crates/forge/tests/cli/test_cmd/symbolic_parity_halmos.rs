use super::symbolic_helpers::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// MakerDAO MiniVat — sanity-check that a correct invariant PROVES.
// ---------------------------------------------------------------------------
// Bounded version of https://github.com/a16z/halmos/blob/main/examples/invariants/src/MiniVat.sol
// Here the invariant `debt == sum(urns)` truly holds; symbolic execution must
// prove (not falsify) it within the invariant_depth budget.
forgetest_init!(minivat_invariant_holds, |prj, cmd| {
    skip_unless_z3!("minivat_invariant_holds");

    prj.add_test(
        "MiniVatHolds.t.sol",
        r#"
import "forge-std/Test.sol";

contract MiniVat {
    // Single-account simplification of the MakerDAO Vat for CI-bounded symbolic
    // proof. The invariant `urn == debt` should hold trivially because every
    // mutation moves them by the same amount, with no path to divergence.
    uint256 public urn;
    uint256 public debt;

    function frob(uint8 amt) external {
        unchecked {
            urn += amt;
            debt += amt;
        }
    }

    function repay(uint8 amt) external {
        if (urn < amt) return;
        unchecked {
            urn -= amt;
            debt -= amt;
        }
    }
}

contract MiniVatHolds is Test {
    MiniVat vat;

    function setUp() public {
        vat = new MiniVat();
        targetContract(address(vat));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    /// forge-config: default.symbolic.width = 2048
    function invariant_debtEqualsUrn() public view {
        assertEq(vat.urn(), vat.debt());
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_debtEqualsUrn"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/MiniVatHolds.t.sol:MiniVatHolds
[PASS] invariant_debtEqualsUrn() ([METRICS])
...
"#]]);
});
