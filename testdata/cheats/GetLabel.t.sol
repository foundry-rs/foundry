// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Vm.sol";

contract GetLabelTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetLabel() public {
        // Label an address.
        vm.label(address(1), "Sir Address the 1st");

        // Retrieve the label and check it.
        string memory label = vm.getLabel(address(1));
        assertEq(label, "Sir Address the 1st");
    }
}
