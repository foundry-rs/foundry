// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/2984
contract Issue2984Test is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);
    uint256 fork;
    uint256 snapshot;

    function setUp() public {
        fork = VM.createSelectFork("avaxTestnet", 12880747);
        snapshot = VM.snapshotState();
    }

    function testForkRevertSnapshot() public {
        VM.revertToState(snapshot);
    }

    function testForkSelectSnapshot() public {
        uint256 fork2 = VM.createSelectFork("avaxTestnet", 12880749);
    }
}
