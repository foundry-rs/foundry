use super::*;
use foundry_config::ProfitOracleMode;

/// A token with a public faucet `mint`, plus vaults built on it. `DonationVault` mints the first
/// deposit 1:1 and prices later shares from its token balance, so a donation can make a later
/// depositor's shares round down. `FeeVault` uses internal accounting and pays fees to a treasury.
/// `YieldVault` mints yield on an index that anyone can raise. `OpenRedeemVault` lets anyone redeem
/// the shares of another depositor. `RewardPool` mints rewards to stakers over time.
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

    function convertToAssets(uint256 shares) external view virtual returns (uint256) {
        return shares;
    }

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

    function convertToAssets(uint256 shares) external view override returns (uint256) {
        return totalSupply == 0 ? shares : shares * asset.balanceOf(address(this)) / totalSupply;
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

contract YieldVault is Vault {
    uint256 public index = 1e18;
    mapping(address => uint256) public userIndex;

    constructor(Token asset_) Vault(asset_) {}

    function deposit(uint256 assets) external override returns (uint256 shares) {
        claim();
        shares = assets;
        asset.transferFrom(msg.sender, address(this), assets);
        _mint(msg.sender, shares);
    }

    function redeem(uint256 shares) external override returns (uint256 assets) {
        claim();
        if (shares > balanceOf[msg.sender]) shares = balanceOf[msg.sender];
        assets = shares;
        _burn(msg.sender, shares);
        asset.transfer(msg.sender, assets);
    }

    function raiseIndex(uint256 bps) external {
        index += index * (bps % 1000) / 10_000;
    }

    function claim() public {
        uint256 owed = balanceOf[msg.sender] * (index - userIndex[msg.sender]) / 1e18;
        userIndex[msg.sender] = index;
        if (owed > 0) asset.mint(msg.sender, owed);
    }
}

contract OpenRedeemVault is Vault {
    constructor(Token asset_) Vault(asset_) {}

    function deposit(uint256 assets) external override returns (uint256 shares) {
        shares = assets;
        asset.transferFrom(msg.sender, address(this), assets);
        _mint(msg.sender, shares);
    }

    function redeem(uint256 shares) external override returns (uint256 assets) {
        return redeemFrom(msg.sender, shares);
    }

    // Bug: no allowance check, so anyone can redeem the shares of another depositor.
    function redeemFrom(address owner, uint256 shares) public returns (uint256 assets) {
        assets = shares;
        _burn(owner, shares);
        asset.transfer(msg.sender, assets);
    }
}

contract RewardPool {
    Token public immutable stakeToken;
    Token public immutable rewardToken;
    mapping(address => uint256) public staked;
    mapping(address => uint256) public lastClaim;

    constructor(Token stakeToken_, Token rewardToken_) {
        stakeToken = stakeToken_;
        rewardToken = rewardToken_;
    }

    function stake(uint256 amount) external {
        claim();
        stakeToken.transferFrom(msg.sender, address(this), amount);
        staked[msg.sender] += amount;
    }

    function unstake(uint256 amount) external {
        claim();
        if (amount > staked[msg.sender]) amount = staked[msg.sender];
        staked[msg.sender] -= amount;
        stakeToken.transfer(msg.sender, amount);
    }

    function claim() public {
        uint256 owed = staked[msg.sender] * (block.timestamp - lastClaim[msg.sender]) / 1 days;
        lastClaim[msg.sender] = block.timestamp;
        if (owed > 0) rewardToken.mint(msg.sender, owed);
    }
}

abstract contract VaultTest is Test {
    Token token;
    Vault vault;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address treasury = makeAddr("treasury");

    function newVault() internal virtual returns (Vault);

    function vaultSelectors() internal virtual returns (bytes4[] memory selectors) {
        selectors = new bytes4[](2);
        selectors[0] = Vault.deposit.selector;
        selectors[1] = Vault.redeem.selector;
    }

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
        targetSelector(FuzzSelector({addr: address(vault), selectors: vaultSelectors()}));
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

contract YieldVaultTest is VaultTest {
    function newVault() internal override returns (Vault) {
        return new YieldVault(token);
    }

    function vaultSelectors() internal override returns (bytes4[] memory selectors) {
        selectors = new bytes4[](4);
        selectors[0] = Vault.deposit.selector;
        selectors[1] = Vault.redeem.selector;
        selectors[2] = YieldVault.raiseIndex.selector;
        selectors[3] = YieldVault.claim.selector;
    }

    function invariant_noop() public view {}
}

contract OpenRedeemVaultTest is VaultTest {
    function newVault() internal override returns (Vault) {
        return new OpenRedeemVault(token);
    }

    function vaultSelectors() internal override returns (bytes4[] memory selectors) {
        selectors = new bytes4[](2);
        selectors[0] = Vault.deposit.selector;
        selectors[1] = OpenRedeemVault.redeemFrom.selector;
    }

    function invariant_noop() public view {}
}

contract RewardPoolTest is Test {
    Token stakeToken;
    Token rewardToken;
    RewardPool pool;
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");

    function setUp() public {
        stakeToken = new Token();
        rewardToken = new Token();
        pool = new RewardPool(stakeToken, rewardToken);
        address[2] memory users = [alice, bob];
        for (uint256 i; i < users.length; i++) {
            stakeToken.mint(users[i], type(uint128).max);
            vm.prank(users[i]);
            stakeToken.approve(address(pool), type(uint256).max);
            targetSender(users[i]);
        }
        bytes4[] memory poolSelectors = new bytes4[](3);
        poolSelectors[0] = RewardPool.stake.selector;
        poolSelectors[1] = RewardPool.unstake.selector;
        poolSelectors[2] = RewardPool.claim.selector;
        targetSelector(FuzzSelector({addr: address(pool), selectors: poolSelectors}));
        bytes4[] memory tokenSelectors = new bytes4[](1);
        tokenSelectors[0] = Token.transfer.selector;
        targetSelector(FuzzSelector({addr: address(stakeToken), selectors: tokenSelectors}));
    }

    function invariant_noop() public view {}
}
"#;

// The oracle catches a share-inflation attack without a user invariant: the attacker donates to an
// empty vault, deposits 1, and takes part of a later deposit that rounds down. The later depositor
// is the victim. The same campaign passes when the oracle is off.
#[forgetest_init]
fn profit_oracle_catches_donation_theft(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(6));
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
[FAIL: 0x1D96F2f6BeF1202E4Ce1Ff6Dad0c2CB002861d3e ended with more value than it put in: 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f +22 (start 340282366920938463463374607431768211455, end 340282366920938463463374607431768211477, outside 0); victim 0xf43Bca55E8091977223Fa5b776E23528D205dcA8 lost value: 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f -22 (start 340282366920938463463374607431768211455, end 340282366920938463463374607431768211433, outside 0)] attacker_profit
	[Sequence] (original: 5, shrunk: 4)
		sender=0x1D96F2f6BeF1202E4Ce1Ff6Dad0c2CB002861d3e addr=[test/ProfitOracle.t.sol:Token]0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f calldata=transfer(address,uint256) args=[0x2e234DAe75C793f67A35089C9d99245E1C58470b, 808]
		sender=0x1D96F2f6BeF1202E4Ce1Ff6Dad0c2CB002861d3e addr=[test/ProfitOracle.t.sol:DonationVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=deposit(uint256) args=[1]
		sender=0xf43Bca55E8091977223Fa5b776E23528D205dcA8 addr=[test/ProfitOracle.t.sol:DonationVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=deposit(uint256) args=[10000 [1e4]]
		sender=0x1D96F2f6BeF1202E4Ce1Ff6Dad0c2CB002861d3e addr=[test/ProfitOracle.t.sol:DonationVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=redeem(uint256) args=[1]
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

// Yield that an index raise mints from nothing has no victim, so it is no attacker profit. The old
// rule (`profit_oracle_mode = "any"`) still reports it.
#[forgetest_init]
fn profit_oracle_is_quiet_on_index_yield(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1));
        config.invariant.runs = 500;
        config.invariant.depth = 10;
        config.invariant.profit.profit_oracle = true;
    });
    prj.add_test("ProfitOracle.t.sol", PROFIT_VAULTS);

    cmd.args(["test", "--mc", "YieldVaultTest"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_noop() (runs: 500, calls: 5000, reverts: [..])
...
"#]]);

    prj.update_config(|config| {
        config.invariant.profit.profit_oracle_mode = ProfitOracleMode::Any;
    });
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: [..] ended with more value than it put in: [..]] attacker_profit
...
"#]]);
}

// A reward that the pool mints to stakers over time has no victim, so it is no attacker profit.
// Stakes in the pool have no price, so they are no loss either. The old rule still reports the
// reward.
#[forgetest_init]
fn profit_oracle_is_quiet_on_reward_emission(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1));
        config.invariant.runs = 500;
        config.invariant.depth = 10;
        config.invariant.max_time_delay = Some(86400);
        config.invariant.profit.profit_oracle = true;
    });
    prj.add_test("ProfitOracle.t.sol", PROFIT_VAULTS);

    cmd.args(["test", "--mc", "RewardPoolTest"]).assert_success().stdout_eq(str![[r#"
...
[PASS] invariant_noop() (runs: 500, calls: 5000, reverts: [..])
...
"#]]);

    prj.update_config(|config| {
        config.invariant.profit.profit_oracle_mode = ProfitOracleMode::Any;
    });
    cmd.assert_failure().stdout_eq(str![[r#"
...
[FAIL: [..] ended with more value than it put in: [..]] attacker_profit
...
"#]]);
}

// A depositor that redeems the shares of another depositor profits, and the other depositor is the
// victim.
#[forgetest_init]
fn profit_oracle_catches_theft_from_other_depositor(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.fuzz.seed = Some(U256::from(1));
        config.invariant.runs = 500;
        config.invariant.depth = 10;
        config.invariant.profit.profit_oracle = true;
    });
    prj.add_test("ProfitOracle.t.sol", PROFIT_VAULTS);

    cmd.args(["test", "--mc", "OpenRedeemVaultTest"]).assert_failure().stdout_eq(str![[r#"
...
Ran 1 test for test/ProfitOracle.t.sol:OpenRedeemVaultTest
[FAIL: 0xf43Bca55E8091977223Fa5b776E23528D205dcA8 ended with more value than it put in: 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f +1 (start 340282366920938463463374607431768211455, end 340282366920938463463374607431768211456, outside 0); victim 0x328809Bc894f92807417D2dAD6b7C998c1aFdac6 lost value: 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f -1 (start 340282366920938463463374607431768211455, end 340282366920938463463374607431768211454, outside 0)] attacker_profit
	[Sequence] (original: 7, shrunk: 2)
		sender=0x328809Bc894f92807417D2dAD6b7C998c1aFdac6 addr=[test/ProfitOracle.t.sol:OpenRedeemVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=deposit(uint256) args=[1560154909105 [1.56e12]]
		sender=0xf43Bca55E8091977223Fa5b776E23528D205dcA8 addr=[test/ProfitOracle.t.sol:OpenRedeemVault]0x2e234DAe75C793f67A35089C9d99245E1C58470b calldata=redeemFrom(address,uint256) args=[0x328809Bc894f92807417D2dAD6b7C998c1aFdac6, 1]
 invariant_noop() (runs: 500, calls: 5000, reverts: [..])
...
"#]]);
}
