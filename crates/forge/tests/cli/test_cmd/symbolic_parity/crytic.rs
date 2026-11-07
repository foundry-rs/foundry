use super::json_test_result;
use crate::skip_unless_z3;
use foundry_test_utils::forgetest_init;

// ---------------------------------------------------------------------------
// crytic/properties ERC20 — sum-of-balances invariant on a BUGGY token.
// ---------------------------------------------------------------------------
// The reference invariant from https://github.com/crytic/properties is
// `sum(balanceOf) == totalSupply`. We use a buggy `_transfer` that mints on
// transfer to a specific address — symbolic execution should expose it.
#[forgetest_init]
fn crytic_properties_erc20_sum_invariant_buggy(prj: _, cmd: _) {
    skip_unless_z3!("crytic_properties_erc20_sum_invariant_buggy");
    // A concrete fuzz fallback must not satisfy this symbolic parity test.
    prj.update_config(|config| config.invariant.runs = 0);

    prj.add_test(
        "CryticPropertiesErc20Parity.t.sol",
        r#"
import "forge-std/Test.sol";

contract BuggyToken {
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply;
    address constant TRACKED_A = address(0xA11CE);
    address constant TRACKED_B = address(0xB0B);

    constructor() {
        balanceOf[TRACKED_A] = 50;
        balanceOf[TRACKED_B] = 50;
        totalSupply = 100;
    }

    function transfer(address to, uint256 amount) external {
        // Keep transfers inside the holder set summed by the invariant.
        if (to != TRACKED_A && to != TRACKED_B) return;
        if (balanceOf[msg.sender] < amount) return;
        unchecked {
            balanceOf[msg.sender] -= amount;
        }
        // Concrete keys avoid abstract initial values for symbolic mapping reads.
        // BUG: doubles the credit when `to` is a specific address.
        if (to == TRACKED_A) {
            balanceOf[TRACKED_A] += amount * 2;
        } else {
            balanceOf[TRACKED_B] += amount;
        }
    }
}

contract CryticPropertiesErc20Parity is Test {
    BuggyToken token;
    address constant TRACKED_A = address(0xA11CE);
    address constant TRACKED_B = address(0xB0B);

    function setUp() public {
        token = new BuggyToken();
        targetContract(address(token));
        targetSender(TRACKED_A);
        targetSender(TRACKED_B);
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_sumOfBalances() public view {
        assertEq(token.balanceOf(TRACKED_A) + token.balanceOf(TRACKED_B), token.totalSupply());
    }
}
"#,
    );

    let output = cmd
        .args(["test", "--symbolic", "--json", "--match-test", "invariant_sumOfBalances"])
        .assert_failure()
        .get_output()
        .stdout
        .clone();
    let result = json_test_result(&output, "invariant_sumOfBalances()");
    assert_eq!(result["symbolic"]["status"], "fail_counterexample", "{result}");
    assert_eq!(result["symbolic"]["replay"]["status"], "confirmed", "{result}");
    let sequence =
        result["invariant_failures"][0]["counterexample"]["Sequence"][1].as_array().unwrap();
    assert_eq!(sequence.len(), 1, "{result}");
}
