// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract BadSigAfterInvariant is DSTest {
    function afterinvariant() public {}

    function testShouldPassWithWarning() public {
        assert(true);
    }
}
