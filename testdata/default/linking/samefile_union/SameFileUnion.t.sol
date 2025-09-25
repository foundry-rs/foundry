// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.18;

import "./Libs.sol";

contract UsesBoth {
    uint256 public x;

    constructor() {
        // used only in creation bytecode
        x = LInit.f();
    }

    function y() external view returns (uint256) {
        // used only in deployed bytecode
        return LRun.g();
    }
}
