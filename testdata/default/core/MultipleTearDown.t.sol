// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract MultipleTearDown is DSTest {
    function tearDown() public {}

    function teardown() public {}

    function testFailShouldBeMarkedAsFailedBecauseOfTeardown() public {
        assert(true);
    }
}
