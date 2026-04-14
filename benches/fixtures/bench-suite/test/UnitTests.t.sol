// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {ERC20} from "../src/ERC20.sol";
import {Vault} from "../src/Vault.sol";
import {Registry} from "../src/Registry.sol";

/// @notice Deterministic unit tests exercising basic test runner speed / TTFB.
contract UnitTests {
    ERC20 token;
    ERC20 tokenA;
    ERC20 tokenB;
    Vault vault;
    Registry registry;

    function setUp() public {
        token = new ERC20("Test", "TST", 18);
        tokenA = new ERC20("TokenA", "TKA", 18);
        tokenB = new ERC20("TokenB", "TKB", 18);
        vault = new Vault(tokenA, tokenB);
        registry = new Registry();
    }

    // --- ERC20 ---

    function test_mint() public {
        token.mint(address(1), 1000e18);
        assert(token.balanceOf(address(1)) == 1000e18);
        assert(token.totalSupply() == 1000e18);
    }

    function test_transfer() public {
        token.mint(address(this), 100e18);
        token.transfer(address(1), 40e18);
        assert(token.balanceOf(address(this)) == 60e18);
        assert(token.balanceOf(address(1)) == 40e18);
    }

    function test_approve_transferFrom() public {
        token.mint(address(this), 100e18);
        token.approve(address(1), 50e18);
        assert(token.allowance(address(this), address(1)) == 50e18);
    }

    function test_burn() public {
        token.mint(address(this), 100e18);
        token.burn(address(this), 30e18);
        assert(token.balanceOf(address(this)) == 70e18);
        assert(token.totalSupply() == 70e18);
    }

    // --- Registry ---

    function test_register() public {
        registry.register(bytes32("key1"), 42);
        (address owner, uint256 value,, bool active) = registry.entries(bytes32("key1"));
        assert(owner == address(this));
        assert(value == 42);
        assert(active);
        assert(registry.totalEntries() == 1);
    }

    function test_update() public {
        registry.register(bytes32("key2"), 10);
        registry.update(bytes32("key2"), 20);
        (, uint256 value,,) = registry.entries(bytes32("key2"));
        assert(value == 20);
    }

    function test_deactivate() public {
        registry.register(bytes32("key3"), 10);
        registry.deactivate(bytes32("key3"));
        (,,, bool active) = registry.entries(bytes32("key3"));
        assert(!active);
    }

    function test_batchRegister() public {
        bytes32[] memory keys = new bytes32[](3);
        uint256[] memory values = new uint256[](3);
        keys[0] = bytes32("a");
        values[0] = 1;
        keys[1] = bytes32("b");
        values[1] = 2;
        keys[2] = bytes32("c");
        values[2] = 3;
        registry.batchRegister(keys, values);
        assert(registry.totalEntries() == 3);
    }

    // --- Vault ---

    function test_deposit() public {
        tokenA.mint(address(this), 100e18);
        tokenB.mint(address(this), 200e18);
        tokenA.approve(address(vault), 100e18);
        tokenB.approve(address(vault), 200e18);

        uint256 minted = vault.deposit(100e18, 200e18);
        assert(minted > 0);
        assert(vault.reserveA() == 100e18);
        assert(vault.reserveB() == 200e18);
    }

    function test_depositWithdraw() public {
        tokenA.mint(address(this), 100e18);
        tokenB.mint(address(this), 100e18);
        tokenA.approve(address(vault), 100e18);
        tokenB.approve(address(vault), 100e18);

        uint256 minted = vault.deposit(100e18, 100e18);
        vault.withdraw(minted);

        assert(vault.totalShares() == 0);
        assert(vault.reserveA() == 0);
        assert(vault.reserveB() == 0);
    }

    function test_swap() public {
        tokenA.mint(address(this), 200e18);
        tokenB.mint(address(this), 200e18);
        tokenA.approve(address(vault), type(uint256).max);
        tokenB.approve(address(vault), type(uint256).max);

        vault.deposit(100e18, 100e18);
        uint256 out = vault.swap(true, 10e18);
        assert(out > 0);
        assert(vault.swapCount() == 1);
    }
}
