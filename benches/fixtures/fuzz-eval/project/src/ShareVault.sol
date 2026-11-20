// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Minimal mintable token used as the vault asset.
contract Asset {
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
    }

    function approve(address spender, uint256 amount) external {
        allowance[msg.sender][spender] = amount;
    }

    function transfer(address to, uint256 amount) external {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
    }

    function transferFrom(address from, address to, uint256 amount) external {
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
    }
}

/// Share-based vault over `Asset`.
contract ShareVault {
    Asset public immutable asset;
    mapping(address => uint256) public balanceOf;
    uint256 public totalSupply;

    constructor(Asset asset_) {
        asset = asset_;
    }

    function totalAssets() public view returns (uint256) {
        return asset.balanceOf(address(this));
    }

    function convertToAssets(uint256 shares) public view returns (uint256) {
        uint256 supply = totalSupply;
        return supply == 0 ? shares : shares * totalAssets() / supply;
    }

    function deposit(uint256 assets) external returns (uint256 shares) {
        uint256 supply = totalSupply;
        // BUG: rounds down with no virtual shares or minimum-shares check, so a
        // donation that inflates the share price can mint depositors nothing.
        shares = supply == 0 ? assets : assets * supply / totalAssets();
        asset.transferFrom(msg.sender, address(this), assets);
        balanceOf[msg.sender] += shares;
        totalSupply += shares;
    }

    function redeem(uint256 shares) external returns (uint256 assets) {
        assets = convertToAssets(shares);
        balanceOf[msg.sender] -= shares;
        totalSupply -= shares;
        asset.transfer(msg.sender, assets);
    }
}
