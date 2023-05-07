// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract PrevrandaoTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testPrevrandao() public {
        assertEq(block.prevrandao, 0);
        cheats.prevrandao(bytes32(uint256(10)));
        assertEq(block.prevrandao, 10, "prevrandao cheatcode failed");
    }

    function testPrevrandaoFuzzed(uint256 newPrevrandao) public {
        cheats.assume(newPrevrandao != block.prevrandao);
        assertEq(block.prevrandao, 0);
        cheats.prevrandao(bytes32(newPrevrandao));
        assertEq(block.prevrandao, newPrevrandao);
    }

    function testPrevrandaoSnapshotFuzzed(uint256 newPrevrandao) public {
        cheats.assume(newPrevrandao != block.prevrandao);
        uint256 oldPrevrandao = block.prevrandao;
        uint256 snapshot = cheats.snapshot();

        cheats.prevrandao(bytes32(newPrevrandao));
        assertEq(block.prevrandao, newPrevrandao);

        assert(cheats.revertTo(snapshot));
        assertEq(block.prevrandao, oldPrevrandao);
    }
}
