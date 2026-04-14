// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {ERC20} from "../src/ERC20.sol";
import {Vault} from "../src/Vault.sol";

interface Vm {
    function startPrank(address) external;
    function stopPrank() external;
}

contract FuzzVaultTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ERC20 tokenA;
    ERC20 tokenB;
    Vault vault;
    address alice = address(0xA11CE);

    function setUp() public {
        tokenA = new ERC20("TokenA", "TKA", 18);
        tokenB = new ERC20("TokenB", "TKB", 18);
        vault = new Vault(tokenA, tokenB);
    }

    function testFuzz_depositWithdraw(uint256 amountA, uint256 amountB) public {
        amountA = (amountA % 1e24) + 1e6;
        amountB = (amountB % 1e24) + 1e6;

        tokenA.mint(alice, amountA);
        tokenB.mint(alice, amountB);

        vm.startPrank(alice);
        tokenA.approve(address(vault), amountA);
        tokenB.approve(address(vault), amountB);
        uint256 sharesMinted = vault.deposit(amountA, amountB);
        assert(sharesMinted > 0);

        vault.withdraw(sharesMinted);
        vm.stopPrank();

        assert(vault.totalShares() == 0);
        assert(vault.reserveA() == 0);
        assert(vault.reserveB() == 0);
    }

    function testFuzz_swap(uint256 initA, uint256 initB, uint256 swapAmount) public {
        initA = (initA % 1e24) + 1e18;
        initB = (initB % 1e24) + 1e18;
        swapAmount = (swapAmount % (initA / 10)) + 1;

        tokenA.mint(alice, initA + swapAmount);
        tokenB.mint(alice, initB);

        vm.startPrank(alice);
        tokenA.approve(address(vault), type(uint256).max);
        tokenB.approve(address(vault), type(uint256).max);
        vault.deposit(initA, initB);

        uint256 preA = tokenA.balanceOf(alice);
        uint256 preB = tokenB.balanceOf(alice);

        vault.swap(true, swapAmount);
        vm.stopPrank();

        assert(tokenA.balanceOf(alice) == preA - swapAmount);
        assert(tokenB.balanceOf(alice) > preB);

        // k should not decrease (constant product invariant)
        assert(vault.reserveA() * vault.reserveB() >= initA * initB);
    }

    function testFuzz_multiSwap(uint256 initA, uint256 initB, uint8 numSwaps) public {
        initA = (initA % 1e22) + 1e18;
        initB = (initB % 1e22) + 1e18;
        numSwaps = uint8((uint256(numSwaps) % 10) + 1);

        tokenA.mint(alice, initA * 2);
        tokenB.mint(alice, initB * 2);

        vm.startPrank(alice);
        tokenA.approve(address(vault), type(uint256).max);
        tokenB.approve(address(vault), type(uint256).max);
        vault.deposit(initA, initB);

        uint256 k0 = vault.reserveA() * vault.reserveB();

        for (uint8 i = 0; i < numSwaps; i++) {
            uint256 rA = vault.reserveA();
            uint256 swapAmt = (rA / 100) + 1;
            if (tokenA.balanceOf(alice) >= swapAmt) {
                vault.swap(true, swapAmt);
            }
        }
        vm.stopPrank();

        // k never decreases (fees only increase it)
        assert(vault.reserveA() * vault.reserveB() >= k0);
    }
}
