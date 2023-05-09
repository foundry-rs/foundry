// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract LabelTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testLabel() public {
        cheats.label(address(1), "Sir Address the 1st");
    }
}
