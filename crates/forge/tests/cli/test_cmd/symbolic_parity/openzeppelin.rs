use super::{assert_symbolic, assert_symbolic_witness};
use crate::skip_unless_z3;
use foundry_test_utils::{forgetest_init, str};

// ---------------------------------------------------------------------------
// ERC4626 inflation / first-depositor donation attack.
// ---------------------------------------------------------------------------
// Classic DeFi bug: vault without virtual offset lets attacker donate raw
// assets after depositing 1 wei, so the next depositor's shares round to 0.
forgetest_init!(erc4626_inflation_attack, |prj, cmd| {
    skip_unless_z3!("erc4626_inflation_attack");

    prj.add_test(
        "Erc4626Inflation.t.sol",
        r#"
import "forge-std/Test.sol";

contract BuggyVault {
    uint256 public totalAssets;
    uint256 public totalShares;
    mapping(address => uint256) public sharesOf;

    function deposit(uint256 assets) external returns (uint256 shares) {
        if (totalShares == 0) {
            shares = assets;
        } else {
            // BUG: no virtual-offset, no rounding-up — shares can round to 0.
            shares = (assets * totalShares) / totalAssets;
        }
        sharesOf[msg.sender] += shares;
        totalShares += shares;
        totalAssets += assets;
    }

    function donate(uint256 assets) external {
        totalAssets += assets;
    }
}

contract Erc4626Inflation is Test {
    function checkDepositReturnsShares(uint64 first, uint128 donation, uint64 second) public {
        vm.assume(first > 0);
        vm.assume(second > 0);
        BuggyVault v = new BuggyVault();
        v.deposit(first);
        v.donate(donation);
        uint256 shares = v.deposit(second);
        // Property: any non-zero deposit MUST return non-zero shares.
        assert(shares > 0);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args([
        "test",
        "--symbolic",
        "--match-test",
        "checkDepositReturnsShares",
    ]))
    .failure()
    .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Erc4626Inflation.t.sol:Erc4626Inflation
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkDepositReturnsShares(uint64,uint128,uint64) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

// ---------------------------------------------------------------------------
// ERC20 approve overwrite race — classic non-atomic approve footgun.
// ---------------------------------------------------------------------------
// The canonical front-run sequence is encoded literally in the test body:
// owner `approve(N1)` → spender `transferFrom(N1)` → owner `approve(N2)` →
// spender `transferFrom(N2)`. The engine does NOT discover the interleaving
// itself; it solves for symbolic `N1`, `N2` over this fixed call sequence and
// witnesses the property violation `balanceOf(spender) > max(N1, N2)`.
forgetest_init!(erc20_approve_race, |prj, cmd| {
    skip_unless_z3!("erc20_approve_race");

    prj.add_test(
        "Erc20ApproveRace.t.sol",
        r#"
import "forge-std/Test.sol";

contract SimpleToken {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    constructor(address owner, uint256 supply) {
        balanceOf[owner] = supply;
    }

    function approve(address spender, uint256 amount) external {
        allowance[msg.sender][spender] = amount;
    }

    function transferFrom(address from, address to, uint256 amount) external {
        require(allowance[from][msg.sender] >= amount);
        require(balanceOf[from] >= amount);
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

contract Erc20ApproveRace is Test {
    function checkApproveRace(uint64 n1, uint64 n2) public {
        vm.assume(n1 > 0);
        vm.assume(n2 > 0);
        address owner = address(0xA11CE);
        address spender = address(0xB0B);
        uint256 supply = uint256(n1) + uint256(n2);
        SimpleToken t = new SimpleToken(owner, supply);

        // Owner: approve N1, then later changes to N2.
        // Spender front-runs the second approve and pulls N1, then pulls N2.
        vm.prank(owner);
        t.approve(spender, n1);

        vm.prank(spender);
        t.transferFrom(owner, spender, n1);

        vm.prank(owner);
        t.approve(spender, n2);

        vm.prank(spender);
        t.transferFrom(owner, spender, n2);

        // Property: spender can never pull more than max(N1, N2) in total.
        uint256 maxIntended = n1 > n2 ? n1 : n2;
        assert(t.balanceOf(spender) <= maxIntended);
    }
}
"#,
    );

    assert_symbolic_witness(cmd.args(["test", "--symbolic", "--match-test", "checkApproveRace"]))
        .failure()
        .stdout_eq(str![[r#"
...
Failing tests:
Encountered 1 failing test in test/Erc20ApproveRace.t.sol:Erc20ApproveRace
[FAIL: panic: assertion failed (0x01); counterexample: [CALLDATA] [ARGS]] checkApproveRace(uint64,uint64) ([METRICS])

Encountered a total of 1 failing tests, 0 tests succeeded

Tip: Run `forge test --rerun` to retry only the 1 failed test
Tip: Run `forge test --debug --match-test <TEST_NAME>` to inspect one failing test in the debugger

"#]]);
});

// ---------------------------------------------------------------------------
// ERC721 ownership uniqueness — bounded stateful proof.
// ---------------------------------------------------------------------------
// Two-token minimal NFT: ownership is unique per id and never falls to the
// zero address after mint. Symbolic invariant should PROVE within depth 2.
forgetest_init!(erc721_unique_ownership_passes, |prj, cmd| {
    skip_unless_z3!("erc721_unique_ownership_passes");

    prj.add_test(
        "Erc721Ownership.t.sol",
        r#"
import "forge-std/Test.sol";

contract MiniNft {
    mapping(uint256 => address) public ownerOf;
    address constant A = address(0xA);
    address constant B = address(0xB);

    constructor() {
        ownerOf[1] = A;
        ownerOf[2] = B;
    }

    function transferFrom(uint256 id, address to) external {
        require(id == 1 || id == 2);
        require(to != address(0));
        require(msg.sender == ownerOf[id]);
        ownerOf[id] = to;
    }
}

contract Erc721Ownership is Test {
    MiniNft nft;

    function setUp() public {
        nft = new MiniNft();
        targetContract(address(nft));
    }

    /// forge-config: default.symbolic.invariant_depth = 2
    function invariant_ownerNonZero() public view {
        assertTrue(nft.ownerOf(1) != address(0));
        assertTrue(nft.ownerOf(2) != address(0));
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "invariant_ownerNonZero"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/Erc721Ownership.t.sol:Erc721Ownership
[PASS] invariant_ownerNonZero() ([METRICS])
...
"#]]);
});

// ---------------------------------------------------------------------------
// ERC4626 deposit → withdraw round-trip — single user, no fee.
// ---------------------------------------------------------------------------
// Property: a user who deposits and then withdraws should get back exactly
// what they put in. Engine should PROVE this for an honest 1:1 vault.
forgetest_init!(erc4626_roundtrip_passes, |prj, cmd| {
    skip_unless_z3!("erc4626_roundtrip_passes");

    prj.add_test(
        "Erc4626Roundtrip.t.sol",
        r#"
contract HonestVault {
    // 1:1 vault — no nonlinear share<->asset math so Z3 can prove the
    // round-trip without falling into nonlinear bv-mul/div. Realistic vaults
    // require nonlinear reasoning that Z3 currently times out on (see the
    // mulDiv note above); that variant belongs in a nightly suite.
    mapping(address => uint256) public sharesOf;

    function deposit(uint256 assets) external returns (uint256 shares) {
        shares = assets;
        sharesOf[msg.sender] += shares;
    }

    function withdraw(uint256 shares) external returns (uint256 assets) {
        require(sharesOf[msg.sender] >= shares);
        assets = shares;
        sharesOf[msg.sender] -= shares;
    }
}

contract Erc4626Roundtrip {
    function checkSingleUserRoundtrip(uint64 amount) public {
        if (amount == 0) return;
        HonestVault v = new HonestVault();
        uint256 shares = v.deposit(amount);
        uint256 returned = v.withdraw(shares);
        assert(returned == amount);
    }
}
"#,
    );

    assert_symbolic(cmd.args(["test", "--symbolic", "--match-test", "checkSingleUserRoundtrip"]))
        .success()
        .stdout_eq(str![[r#"
...
Ran 1 test for test/Erc4626Roundtrip.t.sol:Erc4626Roundtrip
[PASS] checkSingleUserRoundtrip(uint64) ([METRICS])
...
"#]]);
});
