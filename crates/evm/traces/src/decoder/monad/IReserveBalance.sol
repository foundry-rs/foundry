// SPDX-License-Identifier: MIT
pragma solidity >=0.8.13 <0.9.0;

// Mirrors category-labs/monad-std/src/interfaces/IReserveBalance.sol for trace decoding.
interface IReserveBalance {
    function dippedIntoReserve() external returns (bool dipped);
}
