// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SortTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testSortCheatcode() public {
        uint256[] memory numbers = new uint256[](3);
        numbers[0] = 3;
        numbers[1] = 1;
        numbers[2] = 2;

        uint256[] memory sortedNumbers = vm.sort(numbers);

        assertEq(sortedNumbers[0], 1);
        assertEq(sortedNumbers[1], 2);
        assertEq(sortedNumbers[2], 3);
    }
}
