// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract MultipleAfterUnitTest is DSTest {
    function afterTest() public {}

    function aftertest() public {}

    function testFailShouldBeMarkedAsFailedBecauseOfMultiAfterUnitTest()
        public
    {
        assert(true);
    }
}
