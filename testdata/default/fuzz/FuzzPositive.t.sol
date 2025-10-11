// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract FuzzPositive is Test {
    function testSuccessChecker(uint256 val) public {
        assertTrue(true);
    }

    function testSuccessChecker2(int256 val) public {
        assert(val == val);
    }

    function testSuccessChecker3(uint32 val) public {
        assert(val + 0 == val);
    }
}
