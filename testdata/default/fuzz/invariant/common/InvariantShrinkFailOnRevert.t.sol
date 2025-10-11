// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "utils/Test.sol";

contract ShrinkFailOnRevert {
    uint256 cond;

    function work(uint256 x) public {
        if (x % 2 != 0 && x < 9000) {
            cond++;
        }
        require(cond < 10, "condition met");
    }
}

contract ShrinkFailOnRevertTest is Test {
    ShrinkFailOnRevert target;

    function setUp() public {
        target = new ShrinkFailOnRevert();
    }

    function invariant_shrink_fail_on_revert() public view {}
}
