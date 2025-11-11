// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract GetBlockNumberTest is Test {
    function testGetBlockNumber() public {
        uint256 height = vm.getBlockNumber();
        assertEq(height, uint256(block.number), "height should be equal to block.number");
    }

    function testGetBlockNumberWithRoll() public {
        vm.roll(10);
        assertEq(vm.getBlockNumber(), 10, "could not get correct block height after roll");
    }

    function testGetBlockNumberWithRollFuzzed(uint32 jump) public {
        uint256 pre = vm.getBlockNumber();
        vm.roll(pre + jump);
        assertEq(vm.getBlockNumber(), pre + jump, "could not get correct block height after roll");
    }
}
