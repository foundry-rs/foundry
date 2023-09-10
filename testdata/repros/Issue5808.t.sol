// SPDX-License-Identifier: MIT
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/5808
contract Issue5808Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testReadInt() public {
        string memory json = '["ffffffff","00000010"]';
        int256[] memory ints = vm.parseJsonIntArray(json, "");
        assertEq(ints[0], 4294967295);
        assertEq(ints[1], 10);
    }
}
