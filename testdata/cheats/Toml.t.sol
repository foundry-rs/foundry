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

    function test_uintArray() public {
        bytes memory data = vm.parseToml(toml, ".uintArray");
        uint256[] memory decodedData = abi.decode(data, (uint256[]));
        assertEq(42, decodedData[0]);
        assertEq(43, decodedData[1]);
    }

    function test_str() public {
        bytes memory data = vm.parseToml(toml, ".str");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_strArray() public {
        bytes memory data = vm.parseToml(toml, ".strArray");
        string[] memory decodedData = abi.decode(data, (string[]));
        assertEq("hai", decodedData[0]);
        assertEq("there", decodedData[1]);
    }

    function test_bool() public {
        bytes memory data = vm.parseToml(toml, ".bool");
        bool decodedData = abi.decode(data, (bool));
        assertTrue(decodedData);
    }

    function test_boolArray() public {
        bytes memory data = vm.parseToml(toml, ".boolArray");
        bool[] memory decodedData = abi.decode(data, (bool[]));
        assertTrue(decodedData[0]);
        assertTrue(!decodedData[1]);
    }

    function test_address() public {
        bytes memory data = vm.parseToml(toml, ".address");
        address decodedData = abi.decode(data, (address));
        assertEq(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, decodedData);
    }

    function test_addressArray() public {
        bytes memory data = vm.parseToml(toml, ".addressArray");
        address[] memory decodedData = abi.decode(data, (address[]));
        assertEq(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, decodedData[0]);
        assertEq(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, decodedData[1]);
    }

    function test_H160ButNotaddress() public {
        string memory data = abi.decode(vm.parseToml(toml, ".H160NotAddress"), (string));
        assertEq("0000000000000000000000000000000000001337", data);
    }

    struct Nested {
        uint256 number;
        string str;
    }

    function test_nestedObject() public {
        bytes memory data = vm.parseToml(toml, ".nestedObject");
        Nested memory nested = abi.decode(data, (Nested));
        assertEq(nested.number, 9223372036854775807); // TOML is limited to 64-bit integers
        assertEq(nested.str, "NEST");
    }

    // Object keys are sorted alphabetically, regardless of input.
    struct Whole {
        string str;
        string[] strArray;
        uint256[] uintArray;
    }

    function test_wholeToml() public {
        // we need to make the path relative to the crate that's running tests for it (forge crate)
        string memory path = "fixtures/Toml/wholeToml.toml";
        console.log(path);
        toml = vm.readFile(path);
        bytes memory data = vm.parseToml(toml);
        Whole memory whole = abi.decode(data, (Whole));
        assertEq(whole.str, "hai");
        assertEq(whole.uintArray[0], 42);
        assertEq(whole.uintArray[1], 43);
        assertEq(whole.strArray[0], "hai");
        assertEq(whole.strArray[1], "there");
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
