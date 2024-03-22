// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/4402
contract Issue4402Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testReadNonEmptyArray() public {
        string memory path = "fixtures/Json/Issue4402.json";
        string memory json = vm.readFile(path);
        address[] memory tokens = vm.parseJsonAddressArray(json, ".tokens");
        assertEq(tokens.length, 1);

        path = "fixtures/Toml/Issue4402.toml";
        string memory toml = vm.readFile(path);
        tokens = vm.parseTomlAddressArray(toml, ".tokens");
        assertEq(tokens.length, 1);
    }

    function testReadEmptyArray() public {
        string memory path = "fixtures/Json/Issue4402.json";
        string memory json = vm.readFile(path);

        // Every one of these used to causes panic
        address[] memory emptyAddressArray = vm.parseJsonAddressArray(json, ".empty");
        bool[] memory emptyBoolArray = vm.parseJsonBoolArray(json, ".empty");
        bytes[] memory emptyBytesArray = vm.parseJsonBytesArray(json, ".empty");
        bytes32[] memory emptyBytes32Array = vm.parseJsonBytes32Array(json, ".empty");
        string[] memory emptyStringArray = vm.parseJsonStringArray(json, ".empty");
        int256[] memory emptyIntArray = vm.parseJsonIntArray(json, ".empty");
        uint256[] memory emptyUintArray = vm.parseJsonUintArray(json, ".empty");

        assertEq(emptyAddressArray.length, 0);
        assertEq(emptyBoolArray.length, 0);
        assertEq(emptyBytesArray.length, 0);
        assertEq(emptyBytes32Array.length, 0);
        assertEq(emptyStringArray.length, 0);
        assertEq(emptyIntArray.length, 0);
        assertEq(emptyUintArray.length, 0);

        path = "fixtures/Toml/Issue4402.toml";
        string memory toml = vm.readFile(path);

        // Every one of these used to causes panic
        emptyAddressArray = vm.parseTomlAddressArray(toml, ".empty");
        emptyBoolArray = vm.parseTomlBoolArray(toml, ".empty");
        emptyBytesArray = vm.parseTomlBytesArray(toml, ".empty");
        emptyBytes32Array = vm.parseTomlBytes32Array(toml, ".empty");
        emptyStringArray = vm.parseTomlStringArray(toml, ".empty");
        emptyIntArray = vm.parseTomlIntArray(toml, ".empty");
        emptyUintArray = vm.parseTomlUintArray(toml, ".empty");

        assertEq(emptyAddressArray.length, 0);
        assertEq(emptyBoolArray.length, 0);
        assertEq(emptyBytesArray.length, 0);
        assertEq(emptyBytes32Array.length, 0);
        assertEq(emptyStringArray.length, 0);
        assertEq(emptyIntArray.length, 0);
        assertEq(emptyUintArray.length, 0);
    }
}
