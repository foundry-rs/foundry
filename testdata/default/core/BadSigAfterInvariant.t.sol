// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract BadSigAfterInvariant is Test {
    function afterinvariant() public {}

    function testShouldPassWithWarning() public {
        assert(true);
    }
}
