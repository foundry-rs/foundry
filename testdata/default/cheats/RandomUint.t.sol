// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RandomUint is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRandomUint() public {
        uint256 rand = vm.randomUint();

        assertTrue(rand > 0);
    }
}
