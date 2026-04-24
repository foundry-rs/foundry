// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// SHOULD PASS: Interface named IERC20 with correct function signatures
interface IERC20 {
    function transfer(address to, uint256 value) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
}
