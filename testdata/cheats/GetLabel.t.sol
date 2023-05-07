// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract GetLabelTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testGetLabel() public {
        // Label an address.
        cheats.label(address(1), "Sir Address the 1st");

        // Retrieve the label and check it.
        string memory label = cheats.getLabel(address(1));
        assertEq(label, "Sir Address the 1st");
    }
}
