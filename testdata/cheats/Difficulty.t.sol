// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract DifficultyTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testDifficulty() public {
        assertEq(block.difficulty, 0);
        cheats.difficulty(10);
        assertEq(block.difficulty, 10, "difficulty cheatcode failed");
    }

    function testDifficultyFuzzed(uint256 newDifficulty) public {
        cheats.assume(newDifficulty != block.difficulty);
        assertEq(block.difficulty, 0);
        cheats.difficulty(newDifficulty);
        assertEq(block.difficulty, newDifficulty);
    }

    function testDifficultySnapshotFuzzed(uint256 newDifficulty) public {
        cheats.assume(newDifficulty != block.difficulty);
        uint256 oldDifficulty = block.difficulty;
        uint256 snapshot = cheats.snapshot();

        cheats.difficulty(newDifficulty);
        assertEq(block.difficulty, newDifficulty);

        assert(cheats.revertTo(snapshot));
        assertEq(block.difficulty, oldDifficulty);
    }
}
