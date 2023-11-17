// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/4832
contract Issue4832Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFailExample() public {
        assertEq(uint256(1), 2);

        vm.expectRevert();
        revert();
    }
}
