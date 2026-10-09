// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/2984
contract Issue2984Test is Test {
    uint256 fork;
    uint256 snapshot;

    function setUp() public {
        fork = vm.createSelectFork("avaxTestnet", 12880747);
        snapshot = vm.snapshotState();
    }

    function testForkRevertSnapshot() public {
        assertTrue(vm.revertToState(snapshot));
        assertEq(vm.activeFork(), fork);
        assertEq(block.number, 12880747);
    }

    function testForkSelectSnapshot() public {
        uint256 fork2 = vm.createSelectFork("avaxTestnet", 12880749);
        assertTrue(fork2 != fork);
        assertEq(vm.activeFork(), fork2);
        assertEq(block.number, 12880749);
        assertTrue(vm.revertToState(snapshot));
        assertEq(vm.activeFork(), fork);
        assertEq(block.number, 12880747);
    }
}

/// forge-config: default.isolate = false
contract Issue2984NonIsolatedTest is Issue2984Test {}
