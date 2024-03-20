// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract AssumeTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testAssume(uint8 x) public {
        vm.assume(x < 2 ** 7);
        assertTrue(x < 2 ** 7, "did not discard inputs");
    }
}
