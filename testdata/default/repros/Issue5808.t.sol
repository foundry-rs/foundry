// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/5808
contract Issue5808Test is Test {
    function testReadInt() public {
        string memory str1 = '["ffffffff","00000010"]';
        vm._expectCheatcodeRevert();
        int256[] memory ints1 = vm.parseJsonIntArray(str1, "");

        string memory str2 = '["0xffffffff","0x00000010"]';
        int256[] memory ints2 = vm.parseJsonIntArray(str2, "");
        assertEq(ints2[0], 0xffffffff);
        assertEq(ints2[1], 16);
    }
}
