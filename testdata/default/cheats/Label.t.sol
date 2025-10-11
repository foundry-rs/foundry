// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract LabelTest is Test {
    function testLabel() public {
        vm.label(address(1), "Sir Address the 1st");
    }
}
