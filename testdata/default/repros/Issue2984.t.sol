// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/2984
contract Issue2984Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 fork;
    uint256 snapshot;

    function setUp() public {
        fork = vm.createSelectFork("avaxTestnet", 12880747);
        snapshot = vm.snapshot();
    }

    function testForkRevertSnapshot() public {
        vm.revertTo(snapshot);
    }

    function testForkSelectSnapshot() public {
        uint256 fork2 = vm.createSelectFork("avaxTestnet", 12880749);
    }
}
