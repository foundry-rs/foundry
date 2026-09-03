// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

library Inner {
    function addOne(uint256 value) public pure returns (uint256) {
        return value + 1;
    }
}

library Outer {
    function addTwo(uint256 value) public pure returns (uint256) {
        return Inner.addOne(value) + 1;
    }
}

contract Consumer {
    function consume(uint256 value) external pure returns (uint256) {
        return Outer.addTwo(value);
    }
}
