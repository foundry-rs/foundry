// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "foundry-std/Test.sol";
import "../src/DeepStack.sol";

contract DeepStackTest is Test {
    DeepStack public ds;

    function setUp() public {
        ds = new DeepStack();
    }

    function testManyVariables() public {
        uint256 res = ds.manyVariables(1);
        assertEq(res, (1+1) + (1+2) + (1+3) + (1+4) + (1+5) + (1+6) + (1+7) + (1+8) + (1+9) + (1+10) + (1+11) + (1+12) + (1+13) + (1+14) + (1+15) + (1+16));
    }
}
