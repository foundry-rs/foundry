// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Config {
    address public test = 0xcBa28b38103307Ec8dA98377ffF9816C164f9AFa;
}

contract TestSetup is Config, DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    // More context: the inheritance order causes the _failed flag
    // that is usually checked on snapshots to be shifted.
    // We now check for keccak256("failed") on the hevm address.
    // This test should succeed.
    function testSnapshotStorageShift() public {
        uint256 snapshotId = vm.snapshotState();

        vm.prank(test);

        vm.revertToState(snapshotId);
    }
}
