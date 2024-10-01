// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3055
contract Issue3055Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_snapshot() external {
        uint256 snapshotId = vm.snapshotState();
        assertEq(uint256(0), uint256(1));
        vm.revertToState(snapshotId);
    }

    function test_snapshot2() public {
        uint256 snapshotId = vm.snapshotState();
        assertTrue(false);
        vm.revertToState(snapshotId);
        assertTrue(true);
    }

    function test_snapshot3(uint256) public {
        vm.expectRevert();
        // Call exposed_snapshot3() using this to perform an external call,
        // so we can properly test for reverts.
        this.exposed_snapshot3();
    }

    function exposed_snapshot3() public {
        uint256 snapshotId = vm.snapshotState();
        assertTrue(false);
        vm.revertToState(snapshotId);
    }
}
