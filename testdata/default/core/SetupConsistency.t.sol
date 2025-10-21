// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SetupConsistencyCheck is Test {
    uint256 two;
    uint256 four;
    uint256 result;

    function setUp() public {
        two = 2;
        four = 4;
        result = 0;
    }

    function testAdd() public {
        assertEq(result, 0);
        result = two + four;
        assertEq(result, 6);
    }

    function testMultiply() public {
        assertEq(result, 0);
        result = two * four;
        assertEq(result, 8);
    }
}
