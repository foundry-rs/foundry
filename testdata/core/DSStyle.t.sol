// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract DSStyleTest is DSTest {
    function testFailingAssertions() public {
        emit log_string("assertionOne");
        assertEq(uint(1), uint(2));
        emit log_string("assertionTwo");
        assertEq(uint(3), uint(4));
        emit log_string("done");
    }
}
