// SPDX-License-Identifier: MIT
pragma solidity >=0.8.0;

import {ERC20} from "./ERC20.sol";

/// @notice AMM-style constant-product vault with deposit/withdraw/swap.
/// Exercises storage-heavy + math-heavy codepaths.
contract Vault {
    ERC20 public immutable tokenA;
    ERC20 public immutable tokenB;

    uint256 public reserveA;
    uint256 public reserveB;
    uint256 public totalShares;
    mapping(address => uint256) public shares;

    uint256 public swapCount;
    uint256 public constant FEE_BPS = 30; // 0.3%

    event Deposit(address indexed user, uint256 amountA, uint256 amountB, uint256 sharesMinted);
    event Withdraw(address indexed user, uint256 amountA, uint256 amountB, uint256 sharesBurned);
    event Swap(address indexed user, bool aToB, uint256 amountIn, uint256 amountOut);

    constructor(ERC20 _tokenA, ERC20 _tokenB) {
        tokenA = _tokenA;
        tokenB = _tokenB;
    }

    function deposit(uint256 amountA, uint256 amountB) external returns (uint256 minted) {
        require(amountA > 0 && amountB > 0, "Vault: zero amount");

        if (totalShares == 0) {
            minted = sqrt(amountA * amountB);
        } else {
            uint256 shareA = (amountA * totalShares) / reserveA;
            uint256 shareB = (amountB * totalShares) / reserveB;
            minted = shareA < shareB ? shareA : shareB;
        }

        require(minted > 0, "Vault: zero shares");

        tokenA.transferFrom(msg.sender, address(this), amountA);
        tokenB.transferFrom(msg.sender, address(this), amountB);

        reserveA += amountA;
        reserveB += amountB;
        totalShares += minted;
        shares[msg.sender] += minted;

        emit Deposit(msg.sender, amountA, amountB, minted);
    }

    function withdraw(uint256 shareAmount) external returns (uint256 amountA, uint256 amountB) {
        require(shareAmount > 0, "Vault: zero shares");
        require(shares[msg.sender] >= shareAmount, "Vault: insufficient shares");

        amountA = (shareAmount * reserveA) / totalShares;
        amountB = (shareAmount * reserveB) / totalShares;

        shares[msg.sender] -= shareAmount;
        totalShares -= shareAmount;
        reserveA -= amountA;
        reserveB -= amountB;

        tokenA.transfer(msg.sender, amountA);
        tokenB.transfer(msg.sender, amountB);

        emit Withdraw(msg.sender, amountA, amountB, shareAmount);
    }

    function swap(bool aToB, uint256 amountIn) external returns (uint256 amountOut) {
        require(amountIn > 0, "Vault: zero input");

        uint256 amountInAfterFee = amountIn * (10000 - FEE_BPS) / 10000;

        if (aToB) {
            amountOut = (amountInAfterFee * reserveB) / (reserveA + amountInAfterFee);
            require(amountOut > 0 && amountOut < reserveB, "Vault: insufficient liquidity");
            tokenA.transferFrom(msg.sender, address(this), amountIn);
            tokenB.transfer(msg.sender, amountOut);
            reserveA += amountIn;
            reserveB -= amountOut;
        } else {
            amountOut = (amountInAfterFee * reserveA) / (reserveB + amountInAfterFee);
            require(amountOut > 0 && amountOut < reserveA, "Vault: insufficient liquidity");
            tokenB.transferFrom(msg.sender, address(this), amountIn);
            tokenA.transfer(msg.sender, amountOut);
            reserveB += amountIn;
            reserveA -= amountOut;
        }

        swapCount++;
        emit Swap(msg.sender, aToB, amountIn, amountOut);
    }

    function sqrt(uint256 x) internal pure returns (uint256 z) {
        if (x == 0) return 0;
        z = x;
        uint256 y = x / 2 + 1;
        while (y < z) {
            z = y;
            y = (x / y + y) / 2;
        }
    }
}
