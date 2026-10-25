use super::*;

/// A token with a public faucet `mint`, plus two vaults built on it. `DonationVault` mints the
/// first deposit 1:1 and prices later shares from its token balance, so the first depositor takes
/// any earlier donation. `FeeVault` uses internal accounting and pays fees to a treasury.
const PROFIT_VAULTS: &str = r#"
import {Test} from "forge-std/Test.sol";

contract Token {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);

    function mint(address to, uint256 amount) public {
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    function approve(address spender, uint256 amount) public returns (bool) {
        allowance[msg.sender][spender] = amount;
        return true;
    }

    function transfer(address to, uint256 amount) public returns (bool) {
        return transferFrom(msg.sender, to, amount);
    }

    function transferFrom(address from, address to, uint256 amount) public returns (bool) {
        if (from != msg.sender) allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
        return true;
    }
}

abstract contract Vault {
    Token public immutable asset;
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply;

    event Transfer(address indexed from, address indexed to, uint256 value);

    constructor(Token asset_) {
        asset = asset_;
    }

    function deposit(uint256 assets) external virtual returns (uint256 shares);

    function redeem(uint256 shares) external virtual returns (uint256 assets);

    function _mint(address to, uint256 shares) internal {
        balanceOf[to] += shares;
        totalSupply += shares;
        emit Transfer(address(0), to, shares);
    }

    function _burn(address from, uint256 shares) internal {
        balanceOf[from] -= shares;
        totalSupply -= shares;
        emit Transfer(from, address(0), shares);
    }
}

contract DonationVault is Vault {
    constructor(Token asset_) Vault(asset_) {}

    function deposit(uint256 assets) external override returns (uint256 shares) {
        shares = totalSupply == 0 ? assets : assets * totalSupply / asset.balanceOf(address(this));
        asset.transferFrom(msg.sender, address(this), assets);
        _mint(msg.sender, shares);
    }

    function redeem(uint256 shares) external override returns (uint256 assets) {
        assets = shares * asset.balanceOf(address(this)) / totalSupply;
        _burn(msg.sender, shares);
        asset.transfer(msg.sender, assets);
    }
}

contract FeeVault is Vault {
    uint256 constant FEE_BPS = 100;
    address public immutable treasury;

    constructor(Token asset_, address treasury_) Vault(asset_) {
        treasury = treasury_;
    }

    function deposit(uint256 assets) external override returns (uint256 shares) {
        uint256 fee = assets * FEE_BPS / 10_000;
        shares = assets - fee;
        asset.transferFrom(msg.sender, address(this), shares);
        asset.transferFrom(msg.sender, treasury, fee);
        _mint(msg.sender, shares);
    }

    function redeem(uint256 shares) external override returns (uint256 assets) {
        uint256 fee = shares * FEE_BPS / 10_000;
        assets = shares - fee;
        _burn(msg.sender, shares);
        asset.transfer(msg.sender, assets);
        asset.transfer(treasury, fee);
    }
}

abstract contract VaultTest is Test {
    Token token;
    Vault vault;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address treasury = makeAddr("treasury");

    function newVault() internal virtual returns (Vault);

    function setUp() public {
        token = new Token();
        vault = newVault();
        address[3] memory users = [alice, bob, treasury];
        for (uint256 i; i < users.length; i++) {
            token.mint(users[i], type(uint128).max);
            vm.prank(users[i]);
            token.approve(address(vault), type(uint256).max);
            targetSender(users[i]);
        }
        bytes4[] memory vaultSelectors = new bytes4[](2);
        vaultSelectors[0] = Vault.deposit.selector;
        vaultSelectors[1] = Vault.redeem.selector;
        targetSelector(FuzzSelector({addr: address(vault), selectors: vaultSelectors}));
        bytes4[] memory tokenSelectors = new bytes4[](2);
        tokenSelectors[0] = Token.transfer.selector;
        tokenSelectors[1] = Token.mint.selector;
        targetSelector(FuzzSelector({addr: address(token), selectors: tokenSelectors}));
    }
}

contract DonationVaultTest is VaultTest {
    function newVault() internal override returns (Vault) {
        return new DonationVault(token);
    }

    /// forge-config: default.invariant.profit_oracle = true
    function invariant_noop() public view {}
}

contract DonationVaultNoOracleTest is VaultTest {
    function newVault() internal override returns (Vault) {
        return new DonationVault(token);
    }

    function invariant_noop() public view {}
}

contract FeeVaultTest is VaultTest {
    function newVault() internal override returns (Vault) {
        return new FeeVault(token, treasury);
    }

    function invariant_noop() public view {}
}
"#;

// The oracle catches the first depositor taking an earlier donation without a user invariant.
// The same campaign passes when the oracle is off.
#[forgetest_init]
fn profit_oracle_catches_donation_theft(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(3));
        config.invariant.runs = 500;
        config.invariant.depth = 10;
    });
    prj.add_test("ProfitOracle.t.sol", PROFIT_VAULTS);

    cmd.args(["test", "--mc", "DonationVaultNoOracleTest"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_noop() (runs: 500, calls: 5000, reverts: [..])
...
"#]]);

    cmd.forge_fuse().args(["test", "--mc", "DonationVaultTest"]).assert_failure().stdout_eq(
        str![[r#"
...
Ran 1 test for test/ProfitOracle.t.sol:DonationVaultTest
[FAIL: 0xf43Bca55E8091977223Fa5b776E23528D205dcA8 ended with more value than it put in: 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f +51187444165594268011602570282231867883 (start 340282366920938463463374607431768211455, end 391469811086532731474977177714000079338, outside 0); 0x2e234DAe75C793f67A35089C9d99245E1C58470b +7472 (start 0, end 7472, outside 0)] attacker_profit
	[Sequence] (original: 6, shrunk: 3)
		sender=0x328809Bc894f92807417D2dAD6b7C998c1aFdac6 addr=[test/ProfitOracle.t.sol:Token]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=transfer(address,uint256) args=[0x2e234DAe75C793f67A35089C9d99245E1C58470b, 340282366920938463463374607431768211455 [3.402e38]]
		sender=0xf43Bca55E8091977223Fa5b776E23528D205dcA8 addr=[test/ProfitOracle.t.sol:DonationVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=deposit(uint256) args=[8795]
		sender=0xf43Bca55E8091977223Fa5b776E23528D205dcA8 addr=[test/ProfitOracle.t.sol:DonationVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=redeem(uint256) args=[1323]
 invariant_noop() (runs: 500, calls: 5000, reverts: [..])
...
"#]],
    );
}

// Fees paid to a treasury that also trades are not attacker profit.
#[forgetest_init]
fn profit_oracle_is_quiet_on_fee_vault(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1));
        config.invariant.runs = 500;
        config.invariant.depth = 10;
        config.invariant.profit.profit_oracle = true;
    });
    prj.add_test("ProfitOracle.t.sol", PROFIT_VAULTS);

    cmd.args(["test", "--mc", "FeeVaultTest"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_noop() (runs: 500, calls: 5000, reverts: [..])
...
"#]]);
}
