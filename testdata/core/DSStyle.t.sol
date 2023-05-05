// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";

contract DSStyleTest is DSTest {
    function testFailingAssertions() public {
        emit log_string("assertionOne");
        assertEq(uint256(1), uint256(2));
        emit log_string("assertionTwo");
        assertEq(uint256(3), uint256(4));
        emit log_string("done");
    }
}
