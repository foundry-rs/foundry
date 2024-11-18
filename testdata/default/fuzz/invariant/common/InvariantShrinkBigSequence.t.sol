// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract ShrinkBigSequence {
    uint256 cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        }
    }

    function checkCond() public view {
        require(cond < 77, "condition met");
    }
}

contract ShrinkBigSequenceTest is DSTest {
    ShrinkBigSequence target;

    function setUp() public {
        target = new ShrinkBigSequence();
    }

    function invariant_shrink_big_sequence() public view {
        target.checkCond();
    }
}
