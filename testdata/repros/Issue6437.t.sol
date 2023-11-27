// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6437
contract Issue6437Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test0() public {
        string memory json = "[]";
        address[] memory arr = vm.parseJsonAddressArray(json, "$");
        assertEq(arr.length, 0);
    }

    function test1() public {
        string memory json = "[\"0x1111111111111111111111111111111111111111\"]";
        address[] memory arr = vm.parseJsonAddressArray(json, "$");
        assertEq(arr.length, 1);
        assertEq(arr[0], 0x1111111111111111111111111111111111111111);
    }

    function test2() public {
        string memory json = "[\"0x1111111111111111111111111111111111111111\",\"0x2222222222222222222222222222222222222222\"]";
        address[] memory arr = vm.parseJsonAddressArray(json, "$");
        assertEq(arr.length, 2);
        assertEq(arr[0], 0x1111111111111111111111111111111111111111);
        assertEq(arr[1], 0x2222222222222222222222222222222222222222);
    }
}
