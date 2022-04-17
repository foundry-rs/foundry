// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract TravelTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testTravel() public {
        cheats.travel(10);
        assertEq(block.chainid, 10, "travel failed");
    }

    function testTravelFuzzed(uint128 jump) public {
        uint pre = block.chainid;
        cheats.travel(block.chainid + jump);
        assertEq(block.chainid, pre + jump, "travel failed");
    }
}
