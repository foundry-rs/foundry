// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.18;

import "./Libs.sol";

contract UsesBoth {
    uint public x;
    constructor() {
        // used only in в creation bytecode
        x = LInit.f();
    }
    function y() external pure returns (uint) {
        // used only in deployed bytecode
        return LRun.g();
    }
}
