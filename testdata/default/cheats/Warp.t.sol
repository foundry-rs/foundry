// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract WarpTest is Test {
    function testWarp() public {
        vm.warp(10);
        assertEq(vm.getBlockTimestamp(), 10, "warp failed");
    }

    function testWarpFuzzed(uint32 jump) public {
        uint256 pre = vm.getBlockTimestamp();
        vm.warp(vm.getBlockTimestamp() + jump);
        assertEq(vm.getBlockTimestamp(), pre + jump, "warp failed");
    }

    function testWarp2() public {
        assertEq(vm.getBlockTimestamp(), 1);
        vm.warp(100);
        assertEq(vm.getBlockTimestamp(), 100);
    }
}
