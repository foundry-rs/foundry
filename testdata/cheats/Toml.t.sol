// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";
import "../logs/console.sol";

contract ParseTomlTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    string toml;

    function setUp() public {
        string memory path = "fixtures/Toml/test.toml";
        toml = vm.readFile(path);
    }

    struct NestedObject {
        uint256 number;
        string str;
    }

    struct AdvancedJsonPath {
        uint256 id;
    }

    struct Whole {
        string stringValue;
        string stringValue2;
        // string[] stringArray;
        // bool boolValue;
        // bool[] boolArray;
        // bool booleanString;
        // bool[] booleanArray;
        // address addressValue;
        // address[] addressArray;
        // string H160NotAddress;
        // bytes[] bytesArray;
        // uint256 hexUint;
        // string stringUint;
        // uint256 numberUint;
        // uint256[] arrayUint;
        // uint256[] arrayStringUint;
        // int256 stringInt;
        // int256 numberInt;
        // int256 hexInt;
        // NestedObject nestedObject;
        // AdvancedJsonPath[] advancedJsonPath;
    }

    function test_wholeToml() public {
        bytes memory data = vm.parseToml(toml);
        Whole memory whole = abi.decode(data, (Whole));

        assertEq(whole.stringValue, "hai");
        assertEq(whole.stringValue2, "there");
        // assertEq(whole.stringArray[0], "hai");
        // assertEq(whole.stringArray[1], "there");
        // assertEq(whole.boolValue, true);
        // assertEq(whole.boolArray[0], true);
        // assertEq(whole.boolArray[1], false);
        // booleanString
        // bool[] booleanArray;
        // assertEq(whole.addressValue, 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
        // assertEq(whole.addressArray[0], 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266);
        // assertEq(whole.addressArray[1], 0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
        // assertEq(whole.H160NotAddress, "0000000000000000000000000000000000001337");
        // bytes[] bytesArray
        // assertEq(whole.hexUint, "0x12C980");
        // assertEq(whole.stringUint, "9223372036854775807");
        // assertEq(whole.numberUint, 9223372036854775807);
        // assertEq(whole.arrayUint[0], 42);
        // assertEq(whole.arrayUint[1], 43);
        // assertEq(whole.arrayStringUint[0], 42);
        // assertEq(whole.arrayStringUint[1], 43);
        // assertEq(whole.arrayStringUint[2], 0x1231232);
    }
}

contract WriteTomlTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function setUp() public {}

    struct simpleJson {
        uint256 a;
        string b;
    }

    function test_serializeSimpleToml() public {
        string memory json = "json";
        string memory path = "fixtures/Toml/write_simple_test.toml";

        vm.serializeUint(json, "a", uint256(123));
        string memory semiFinal = vm.serializeString(json, "b", "test");
        string memory finalJson = vm.serializeString(json, "c", semiFinal);
        console.log(finalJson);
        vm.writeToml(finalJson, path);
    }
}