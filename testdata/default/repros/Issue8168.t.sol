// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/8168
contract Issue8168Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testForkWarpRollPreserved() public {
        uint256 fork1 = vm.createFork("mainnet");
        uint256 fork2 = vm.createFork("mainnet");

        vm.selectFork(fork1);
        uint256 initial_fork1_number = block.number;
        uint256 initial_fork1_ts = block.timestamp;
        vm.warp(block.timestamp + 1000);
        vm.roll(block.number + 100);
        assertEq(block.timestamp, initial_fork1_ts + 1000);
        assertEq(block.number, initial_fork1_number + 100);

        vm.selectFork(fork2);
        uint256 initial_fork2_number = block.number;
        uint256 initial_fork2_ts = block.timestamp;
        vm.warp(block.timestamp + 2000);
        vm.roll(block.number + 200);
        assertEq(block.timestamp, initial_fork2_ts + 2000);
        assertEq(block.number, initial_fork2_number + 200);

        vm.selectFork(fork1);
        assertEq(block.timestamp, initial_fork1_ts + 1000);
        assertEq(block.number, initial_fork1_number + 100);

        vm.selectFork(fork2);
        assertEq(block.timestamp, initial_fork2_ts + 2000);
        assertEq(block.number, initial_fork2_number + 200);
    }
}
