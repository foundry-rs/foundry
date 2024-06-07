// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract BadSigTearDown is DSTest {
    function teardown() public {}

    function testShouldPassWithWarning() public {
        assert(true);
    }
}
