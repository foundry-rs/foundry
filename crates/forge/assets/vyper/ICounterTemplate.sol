// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

interface ICounter {
    function counter() external view returns (uint256);
    function set_counter(uint256 new_counter) external;
    function increment() external;
}