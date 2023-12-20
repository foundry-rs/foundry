// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract GetBlockHeightTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetBlockHeight() public {
        uint256 height = vm.getBlockHeight();
        assertEq(height, uint256(block.number), "height should be equal to block.number");
    }

    function testGetBlockHeightWithRoll() public {
        vm.roll(10);
        assertEq(vm.getBlockHeight(), 10, "could not get correct block height after roll");
    }

    function testGetBlockHeightWithRollFuzzed(uint128 jump) public {
        uint256 pre = vm.getBlockHeight();
        vm.roll(pre + jump);
        assertEq(vm.getBlockHeight(), pre + jump, "could not get correct block height after roll");
    }
}
