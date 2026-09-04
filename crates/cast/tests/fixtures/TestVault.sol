// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract TestVaultAsset {
    string public name = "Test Vault Asset";
    string public symbol = "TVA";
    uint8 public decimals = 18;
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function transfer(address to, uint256 amount) external returns (bool) {
        _transfer(msg.sender, to, amount);
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        require(allowed >= amount, "Insufficient allowance");
        allowance[from][msg.sender] = allowed - amount;
        _transfer(from, to, amount);
        return true;
    }

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    function _transfer(address from, address to, uint256 amount) internal {
        require(balanceOf[from] >= amount, "Insufficient balance");
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
    }
}

contract TestVault {
    TestVaultAsset public immutable underlying;
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Deposit(address indexed sender, address indexed owner, uint256 assets, uint256 shares);
    event Withdraw(
        address indexed sender,
        address indexed receiver,
        address indexed owner,
        uint256 assets,
        uint256 shares
    );

    constructor() {
        underlying = new TestVaultAsset();
        underlying.mint(msg.sender, 1_000 ether);
    }

    function asset() external view returns (address) {
        return address(underlying);
    }

    function totalAssets() external view returns (uint256) {
        return underlying.balanceOf(address(this));
    }

    function convertToShares(uint256 assets) external pure returns (uint256) {
        return assets;
    }

    function convertToAssets(uint256 shares) external pure returns (uint256) {
        return shares;
    }

    // Conservative zero maxima exercise the CLI's compatibility warnings while writes remain live.
    function maxDeposit(address) external pure returns (uint256) {
        return 0;
    }

    function previewDeposit(uint256 assets) external pure returns (uint256) {
        return assets;
    }

    function deposit(uint256 assets, address receiver) external returns (uint256 shares) {
        shares = assets;
        require(underlying.transferFrom(msg.sender, address(this), assets));
        _mint(receiver, shares);
        emit Deposit(msg.sender, receiver, assets, shares);
    }

    function maxMint(address) external pure returns (uint256) {
        return 0;
    }

    function previewMint(uint256 shares) external pure returns (uint256) {
        return shares;
    }

    function mint(uint256 shares, address receiver) external returns (uint256 assets) {
        assets = shares;
        require(underlying.transferFrom(msg.sender, address(this), assets));
        _mint(receiver, shares);
        emit Deposit(msg.sender, receiver, assets, shares);
    }

    function maxWithdraw(address) external pure returns (uint256) {
        return 0;
    }

    function previewWithdraw(uint256 assets) external pure returns (uint256) {
        return assets;
    }

    function withdraw(uint256 assets, address receiver, address owner)
        external
        returns (uint256 shares)
    {
        shares = assets;
        _spendAllowance(owner, shares);
        _burn(owner, shares);
        require(underlying.transfer(receiver, assets));
        emit Withdraw(msg.sender, receiver, owner, assets, shares);
    }

    function maxRedeem(address) external pure returns (uint256) {
        return 0;
    }

    function previewRedeem(uint256 shares) external pure returns (uint256) {
        return shares;
    }

    function redeem(uint256 shares, address receiver, address owner)
        external
        returns (uint256 assets)
    {
        assets = shares;
        _spendAllowance(owner, shares);
        _burn(owner, shares);
        require(underlying.transfer(receiver, assets));
        emit Withdraw(msg.sender, receiver, owner, assets, shares);
    }

    function _mint(address receiver, uint256 shares) internal {
        totalSupply += shares;
        balanceOf[receiver] += shares;
    }

    function _burn(address owner, uint256 shares) internal {
        require(balanceOf[owner] >= shares, "Insufficient shares");
        balanceOf[owner] -= shares;
        totalSupply -= shares;
    }

    function _spendAllowance(address owner, uint256 shares) internal {
        if (msg.sender != owner) {
            uint256 allowed = allowance[owner][msg.sender];
            require(allowed >= shares, "Insufficient allowance");
            allowance[owner][msg.sender] = allowed - shares;
        }
    }
}
