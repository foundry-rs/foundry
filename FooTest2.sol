// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.1;

contract FooBar {
    uint256 x;

    function setUp() public {
        x = 1;
    }

    function testX() public {
        require(x == 1, "x is not one");
    }
}
