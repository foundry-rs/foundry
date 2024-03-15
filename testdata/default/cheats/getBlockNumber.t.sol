// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetBlockNumberTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetBlockNumber() public {
        uint256 height = vm.getBlockNumber();
        assertEq(height, uint256(block.number), "height should be equal to block.number");
    }

    function testGetBlockNumberWithRoll() public {
        vm.roll(10);
        assertEq(vm.getBlockNumber(), 10, "could not get correct block height after roll");
    }

    function testGetBlockNumberWithRollFuzzed(uint128 jump) public {
        uint256 pre = vm.getBlockNumber();
        vm.roll(pre + jump);
        assertEq(vm.getBlockNumber(), pre + jump, "could not get correct block height after roll");
    }
}
