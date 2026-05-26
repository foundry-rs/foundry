use super::assert_symbolic;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// MakerDAO MiniVat — linear smoke version.
// ---------------------------------------------------------------------------
// Stand-in for https://github.com/a16z/halmos/blob/main/examples/invariants/src/MiniVat.sol
// that drops `rate`/`Art` and reduces the invariant to `urn == debt`. Every
// mutation moves them by the same scalar so the engine has nothing to do
// beyond confirming two identical `+=`/`-=` produce equal state. Kept to
// exercise the symbolic invariant harness on a trivially preserved property;
// the canonical Fundamental Equation of DAI variant lives below.
forgetest_init!(minivat_linear_smoke_parity, |prj, cmd| {
    skip_unless_z3!("minivat_linear_smoke_parity");

    prj.add_test(
        "MiniVatLinear.t.sol",
        r#"
import "forge-std/Test.sol";

contract MiniVat {
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

contract MiniVatLinear is Test {
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
Ran 1 test for test/MiniVatLinear.t.sol:MiniVatLinear
[PASS] invariant_debtEqualsUrn() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// MakerDAO MiniVat — Fundamental Equation of DAI.
// ---------------------------------------------------------------------------
// Faithful port of https://github.com/a16z/halmos/blob/main/examples/invariants/test/MiniVat.t.sol
// proving `debt == Art * rate` across arbitrary interleavings of `init` /
// `frob` / `fold`. This is a nonlinear multi-variable algebraic identity:
// `frob(dart)` adds `dart * rate` to debt, `fold(delta)` mutates `rate` and
// adds `Art * delta` to debt.
//
// Currently `#[ignore]`: this is a nonlinear invariant proof over symbolic
// products. The hard-arithmetic fallback can find concrete counterexamples,
// but it cannot certify that every path is safe. Re-enable when nonlinear
// invariant proofs are supported.
forgetest_init!(
    #[ignore = "engine gap: nonlinear bv-mul (Art * rate, symbolic*symbolic) returns solver unknown"]
    minivat_fundamental_equation_parity,
    |prj, cmd| {
        skip_unless_z3!("minivat_fundamental_equation_parity");

        prj.add_test(
            "MiniVatFundamental.t.sol",
            r#"
import "forge-std/Test.sol";

contract MiniVat {
    uint256 public Art;
    uint256 public rate;
    uint256 public debt;

    function init() public {
        require(rate == 0, "rate not zero");
        rate = 10**27;
    }

    function frob(uint128 dart) public {
        unchecked {
            Art += dart;
            debt += uint256(dart) * rate;
        }
    }

    function fold(uint128 delta) public {
        unchecked {
            rate += delta;
            debt += Art * uint256(delta);
        }
    }
}

contract MiniVatFundamental is Test {
    MiniVat vat;

    function setUp() public {
        vat = new MiniVat();
        targetContract(address(vat));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    /// forge-config: default.symbolic.width = 2048
    function invariant_dai() public view {
        assertEq(vat.debt(), vat.Art() * vat.rate(), "Fundamental Equation of DAI");
    }
}
"#,
        );

        assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_dai"]))
            .failure();
    }
);
