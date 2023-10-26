// SPDX-License-Identifier: MIT
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/5808
contract Issue5808Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testReadInt() public {
        string memory str1 = '["ffffffff","00000010"]';
        vm.expectRevert();
        int256[] memory ints1 = vm.parseJsonIntArray(str1, "");

        string memory str2 = '["0xffffffff","0x00000010"]';
        int256[] memory ints2 = vm.parseJsonIntArray(str2, "");
        assertEq(ints2[0], 0xffffffff);
        assertEq(ints2[1], 0x00000010);
    }
}
