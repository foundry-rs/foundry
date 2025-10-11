// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract AssumeTest is Test {
    function testAssume(uint8 x) public {
        vm.assume(x < 2 ** 7);
        assertTrue(x < 2 ** 7, "did not discard inputs");
    }
}
