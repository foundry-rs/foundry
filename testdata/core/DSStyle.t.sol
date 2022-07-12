// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";

contract DSStyleTest is Test {
    function testFailingAssertions() public {
        emit log_string("assertionOne");
        assertEq(uint(1), uint(2));
        emit log_string("assertionTwo");
        assertEq(uint(3), uint(4));
        emit log_string("done");
    }
}
