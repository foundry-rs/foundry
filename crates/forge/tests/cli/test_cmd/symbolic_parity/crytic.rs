use super::assert_symbolic_witness;
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// crytic/properties ERC20 — sum-of-balances invariant on a BUGGY token.
// ---------------------------------------------------------------------------
// The reference invariant from https://github.com/crytic/properties is
// `sum(balanceOf) == totalSupply`. We use a buggy `_transfer` that mints on
// transfer to a specific address — symbolic execution should expose it.
forgetest_init!(crytic_properties_erc20_sum_invariant_buggy, |prj, cmd| {
    skip_unless_z3!("crytic_properties_erc20_sum_invariant_buggy");

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
        if (balanceOf[msg.sender] < amount) return;
        unchecked {
            balanceOf[msg.sender] -= amount;
        }
        // BUG: doubles the credit when `to` is a specific address.
        if (to == TRACKED_A) {
            balanceOf[to] += amount * 2;
        } else {
            balanceOf[to] += amount;
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

    // Witness sequence picks arbitrary `transfer(address,uint256)` args.
    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "invariant_sumOfBalances",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/CryticPropertiesErc20Parity.t.sol:CryticPropertiesErc20Parity
[FAIL: symbolic invariant counterexample]
	[Sequence] (original: 1, shrunk: 1)
		[SENDER] addr=[test/CryticPropertiesErc20Parity.t.sol:BuggyToken]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=transfer(address,uint256) [ARGS]
 invariant_sumOfBalances() ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test

"#]]);
});
