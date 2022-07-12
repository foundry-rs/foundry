// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "forge-std/Test.sol";
import "./Cheats.sol";

contract LabelTest is Test {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testLabel() public {
        cheats.label(address(1), "Sir Address the 1st");
    }
}
