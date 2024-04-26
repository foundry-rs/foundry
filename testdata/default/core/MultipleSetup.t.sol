// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract MultipleSetup is DSTest {
    function setUp() public {}

    function setup() public {}

    function testFailShouldBeMarkedAsFailedBecauseOfSetup() public {
        assert(true);
    }
}
