// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract GetBlockTimestampTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetTimestamp() public {
        uint256 timestamp = vm.getBlockTimestamp();
        assertEq(timestamp, 1, "timestamp should be 1");
    }

    function testGetTimestampWithWarp() public {
        uint256 timestamp = vm.getBlockTimestamp();
        assertEq(timestamp, 1, "timestamp should be 1");
        vm.warp(10);
        assertEq(block.timestamp, 10, "warp failed");
    }

    function testGetTimestampWithWarpFuzzed(uint128 jump) public {
        uint256 pre = vm.getBlockTimestamp();
        vm.warp(pre + jump);
        assertEq(vm.getBlockTimestamp(), pre + jump, "warp failed");
    }
}
