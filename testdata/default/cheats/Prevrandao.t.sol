// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract PrevrandaoTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testPrevrandao() public {
        assertEq(block.prevrandao, 0);
        vm.prevrandao(uint256(10));
        assertEq(block.prevrandao, 10, "prevrandao cheatcode failed");
    }

    function testPrevrandaoFuzzed(uint256 newPrevrandao) public {
        vm.assume(newPrevrandao != block.prevrandao);
        assertEq(block.prevrandao, 0);
        vm.prevrandao(newPrevrandao);
        assertEq(block.prevrandao, newPrevrandao);
    }

    function testPrevrandaoSnapshotFuzzed(uint256 newPrevrandao) public {
        vm.assume(newPrevrandao != block.prevrandao);
        uint256 oldPrevrandao = block.prevrandao;
        uint256 snapshotId = vm.snapshotState();

        vm.prevrandao(newPrevrandao);
        assertEq(block.prevrandao, newPrevrandao);

        assert(vm.revertToState(snapshotId));
        assertEq(block.prevrandao, oldPrevrandao);
    }
}
