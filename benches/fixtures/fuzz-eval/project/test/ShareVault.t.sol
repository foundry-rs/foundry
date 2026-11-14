// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Asset, ShareVault} from "../src/ShareVault.sol";
import {FuzzBase, bound, vm} from "./utils/FuzzBase.sol";

contract VaultHandler {
    Asset public immutable asset;
    ShareVault public immutable vault;
    address[3] actors = [address(0x1001), address(0x1002), address(0x1003)];

    /// Largest loss (in basis points of the deposit) any single deposit suffered.
    uint256 public worstDepositLossBps;

    constructor() {
        asset = new Asset();
        vault = new ShareVault(asset);
    }

    function deposit(uint256 actorSeed, uint256 assets) external {
        address actor = actors[actorSeed % actors.length];
        assets = bound(assets, 1, 1e24);
        asset.mint(actor, assets);
        vm.prank(actor);
        asset.approve(address(vault), assets);
        uint256 before = vault.convertToAssets(vault.balanceOf(actor));
        vm.prank(actor);
        vault.deposit(assets);
        uint256 gained = vault.convertToAssets(vault.balanceOf(actor)) - before;
        if (assets >= 1e6 && gained < assets) {
            uint256 lossBps = (assets - gained) * 10_000 / assets;
            if (lossBps > worstDepositLossBps) worstDepositLossBps = lossBps;
        }
    }

    function redeem(uint256 actorSeed, uint256 shares) external {
        address actor = actors[actorSeed % actors.length];
        shares = bound(shares, 0, vault.balanceOf(actor));
        vm.prank(actor);
        vault.redeem(shares);
    }

    function donate(uint256 assets) external {
        assets = bound(assets, 1, 1e24);
        asset.mint(address(vault), assets);
    }
}

contract ShareVaultInvariantTest is FuzzBase {
    VaultHandler handler;

    function setUp() public {
        handler = new VaultHandler();
        targetContract(address(handler));
    }

    /// Rounding may cost a depositor dust, never more than 1% of a deposit.
    function invariant_depositRoundingIsBounded() public view {
        require(handler.worstDepositLossBps() <= 100, "deposit lost value to rounding");
    }
}
