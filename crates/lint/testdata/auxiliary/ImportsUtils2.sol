// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

function process(bool flag) pure returns (uint256) {
    return flag ? 1 : 0;
}

function calculate(uint256 a, uint256 b) pure returns (uint256) {
    return a + b;
}
