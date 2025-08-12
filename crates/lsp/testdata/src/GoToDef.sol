// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.19;

import {A as AnotherContract, C, D} from "./Deps.sol";

contract GoToDef {
    uint256 public stateVar = 42; // DEFINITION of stateVar (Line 6)

    AnotherContract public another; // DEFINITION of another (Line 8)

    function doSomething() public {
        uint256 local = stateVar; // USAGE of stateVar (Line 11)

        another.add_num(1); // USAGE of add_num (Line 11)
    }
}
