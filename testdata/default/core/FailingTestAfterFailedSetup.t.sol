// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract FailingTestAfterFailedSetupTest is Test {
    function setUp() public {
        assertTrue(false);
    }

    function testAssertSuccess() public {
        assertTrue(true);
    }

    function testAssertFailure() public {
        assertTrue(false);
    }
}
