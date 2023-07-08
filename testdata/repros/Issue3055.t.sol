// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3055
contract Issue3055Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_snapshot() external {
        uint256 snapId = vm.snapshot();
        assertEq(uint256(0), uint256(1));
        vm.revertTo(snapId);
    }

    function test_snapshot2() public {
        uint256 snapshot = vm.snapshot();
        assertTrue(false);
        vm.revertTo(snapshot);
        assertTrue(true);
    }

    function test_snapshot3(uint256) public {
        vm.expectRevert();
        // Call exposed_snapshot3() using this to perform an external call,
        // so we can properly test for reverts.
        this.exposed_snapshot3();
    }

    function exposed_snapshot3() public {
        uint256 snapshot = vm.snapshot();
        assertTrue(false);
        vm.revertTo(snapshot);
    }
}
