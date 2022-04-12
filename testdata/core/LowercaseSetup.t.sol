// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract LowercaseSetup is DSTest {
    uint256 two;

    function setup() public {
        two = 2;
    }

    function testSetup() public {
        assertEq(two, 2);
    }
}
