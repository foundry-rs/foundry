// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {ERC20} from "../src/ERC20.sol";
import {Vault} from "../src/Vault.sol";

interface Vm {
    function prank(address) external;
    function startPrank(address) external;
    function stopPrank() external;
    function targetContract(address) external;
}

contract VaultHandler {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ERC20 public tokenA;
    ERC20 public tokenB;
    Vault public vault;
    address public actor = address(0xBEEF);

    uint256 public ghost_depositA;
    uint256 public ghost_depositB;
    uint256 public ghost_withdrawA;
    uint256 public ghost_withdrawB;

    constructor(ERC20 _a, ERC20 _b, Vault _v) {
        tokenA = _a;
        tokenB = _b;
        vault = _v;
    }

    function deposit(uint256 amountA, uint256 amountB) external {
        amountA = (amountA % 1e22) + 1e6;
        amountB = (amountB % 1e22) + 1e6;

        tokenA.mint(actor, amountA);
        tokenB.mint(actor, amountB);

        vm.startPrank(actor);
        tokenA.approve(address(vault), amountA);
        tokenB.approve(address(vault), amountB);
        vault.deposit(amountA, amountB);
        vm.stopPrank();

        ghost_depositA += amountA;
        ghost_depositB += amountB;
    }

    function withdraw(uint256 sharePercent) external {
        uint256 userShares = vault.shares(actor);
        if (userShares == 0) return;
        sharePercent = (sharePercent % 100) + 1;
        uint256 amount = (userShares * sharePercent) / 100;
        if (amount == 0) return;

        uint256 aOut = (amount * vault.reserveA()) / vault.totalShares();
        uint256 bOut = (amount * vault.reserveB()) / vault.totalShares();

        vm.prank(actor);
        vault.withdraw(amount);

        ghost_withdrawA += aOut;
        ghost_withdrawB += bOut;
    }

    function swap(uint256 amountIn, bool aToB) external {
        if (vault.reserveA() < 1e9 || vault.reserveB() < 1e9) return;

        uint256 maxIn;
        if (aToB) {
            maxIn = vault.reserveA() / 10;
        } else {
            maxIn = vault.reserveB() / 10;
        }
        amountIn = (amountIn % maxIn) + 1;

        if (aToB) {
            tokenA.mint(actor, amountIn);
            vm.startPrank(actor);
            tokenA.approve(address(vault), amountIn);
        } else {
            tokenB.mint(actor, amountIn);
            vm.startPrank(actor);
            tokenB.approve(address(vault), amountIn);
        }

        vault.swap(aToB, amountIn);
        vm.stopPrank();
    }
}

contract InvariantVaultTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    ERC20 tokenA;
    ERC20 tokenB;
    Vault vault;
    VaultHandler handler;

    function setUp() public {
        tokenA = new ERC20("TokenA", "TKA", 18);
        tokenB = new ERC20("TokenB", "TKB", 18);
        vault = new Vault(tokenA, tokenB);
        handler = new VaultHandler(tokenA, tokenB, vault);
        vm.targetContract(address(handler));
    }

    function invariant_reservesMatchBalances() public view {
        assert(vault.reserveA() == tokenA.balanceOf(address(vault)));
        assert(vault.reserveB() == tokenB.balanceOf(address(vault)));
    }

    function invariant_sharesNonNegative() public view {
        assert(vault.totalShares() >= 0);
    }

    function invariant_noSharesImpliesNoReserves() public view {
        if (vault.totalShares() == 0) {
            assert(vault.reserveA() == 0);
            assert(vault.reserveB() == 0);
        }
    }
}
