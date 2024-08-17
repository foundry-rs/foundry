// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract MultipleAfterInvariant is DSTest {
    function afterInvariant() public {}

    function afterinvariant() public {}

    function testFailShouldBeMarkedAsFailedBecauseOfAfterInvariant() public {
        assert(true);
    }
}
