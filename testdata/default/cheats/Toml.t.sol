// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../logs/console.sol";

contract ParseTomlTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    string toml;

    function setUp() public {
        string memory path = "fixtures/Toml/test.toml";
        toml = vm.readFile(path);
    }

    function test_basicString() public {
        bytes memory data = vm.parseToml(toml, ".basicString");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_nullString() public {
        bytes memory data = vm.parseToml(toml, ".nullString");
        string memory decodedData = abi.decode(data, (string));
        assertEq("", decodedData);
    }

    function test_stringMultiline() public {
        bytes memory data = vm.parseToml(toml, ".multilineString");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai\nthere\n", decodedData);
    }

    function test_stringArray() public {
        bytes memory data = vm.parseToml(toml, ".stringArray");
        string[] memory decodedData = abi.decode(data, (string[]));
        assertEq("hai", decodedData[0]);
        assertEq("there", decodedData[1]);
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

    function test_bool() public {
        bytes memory data = vm.parseToml(toml, ".boolTrue");
        bool decodedData = abi.decode(data, (bool));
        assertTrue(decodedData);

        data = vm.parseToml(toml, ".boolFalse");
        decodedData = abi.decode(data, (bool));
        assertTrue(!decodedData);
    }

    function test_boolArray() public {
        bytes memory data = vm.parseToml(toml, ".boolArray");
        bool[] memory decodedData = abi.decode(data, (bool[]));
        assertTrue(decodedData[0]);
        assertTrue(!decodedData[1]);
    }

    function test_dateTime() public {
        bytes memory data = vm.parseToml(toml, ".datetime");
        string memory decodedData = abi.decode(data, (string));
        assertEq(decodedData, "2021-08-10T14:48:00Z");
    }

    function test_dateTimeArray() public {
        bytes memory data = vm.parseToml(toml, ".datetimeArray");
        string[] memory decodedData = abi.decode(data, (string[]));
        assertEq(decodedData[0], "2021-08-10T14:48:00Z");
        assertEq(decodedData[1], "2021-08-10T14:48:00Z");
    }

    function test_uintArray() public {
        bytes memory data = vm.parseToml(toml, ".uintArray");
        uint256[] memory decodedData = abi.decode(data, (uint256[]));
        assertEq(42, decodedData[0]);
        assertEq(43, decodedData[1]);
    }

    // Object keys are sorted alphabetically, regardless of input.
    struct Whole {
        string str;
        string[] strArray;
        uint256[] uintArray;
    }

    function test_wholeToml() public {
        // we need to make the path relative to the crate that's running tests for it (forge crate)
        string memory path = "fixtures/Toml/whole_toml.toml";
        console.log(path);
        toml = vm.readFile(path);
        bytes memory data = vm.parseToml(toml);
        Whole memory whole = abi.decode(data, (Whole));
        assertEq(whole.str, "hai");
        assertEq(whole.strArray[0], "hai");
        assertEq(whole.strArray[1], "there");
        assertEq(whole.uintArray[0], 42);
        assertEq(whole.uintArray[1], 43);
    }

    function test_coercionRevert() public {
        vm._expectCheatcodeRevert("expected uint256, found JSON object");
        vm.parseTomlUint(toml, ".nestedObject");
    }

    function test_coercionUint() public {
        uint256 number = vm.parseTomlUint(toml, ".uintNumber");
        assertEq(number, 9223372036854775807); // TOML is limited to 64-bit integers
        number = vm.parseTomlUint(toml, ".uintString");
        assertEq(number, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
        number = vm.parseTomlUint(toml, ".uintHex");
        assertEq(number, 1231232);
        uint256[] memory numbers = vm.parseTomlUintArray(toml, ".uintArray");
        assertEq(numbers[0], 42);
        assertEq(numbers[1], 43);
        numbers = vm.parseTomlUintArray(toml, ".uintStringArray");
        assertEq(numbers[0], 1231232);
        assertEq(numbers[1], 1231232);
        assertEq(numbers[2], 1231232);
    }

    function test_coercionInt() public {
        int256 number = vm.parseTomlInt(toml, ".intNumber");
        assertEq(number, -12);
        number = vm.parseTomlInt(toml, ".intString");
        assertEq(number, -12);
        number = vm.parseTomlInt(toml, ".intHex");
        assertEq(number, -12);
    }

    function test_coercionBool() public {
        bool boolean = vm.parseTomlBool(toml, ".boolTrue");
        assertTrue(boolean);
        bool boolFalse = vm.parseTomlBool(toml, ".boolFalse");
        assertTrue(!boolFalse);
        boolean = vm.parseTomlBool(toml, ".boolString");
        assertEq(boolean, true);
        bool[] memory booleans = vm.parseTomlBoolArray(toml, ".boolArray");
        assertTrue(booleans[0]);
        assertTrue(!booleans[1]);
        booleans = vm.parseTomlBoolArray(toml, ".boolStringArray");
        assertTrue(booleans[0]);
        assertTrue(!booleans[1]);
    }

    function test_coercionBytes() public {
        bytes memory bytes_ = vm.parseTomlBytes(toml, ".bytesString");
        assertEq(bytes_, hex"01");

        bytes[] memory bytesArray = vm.parseTomlBytesArray(toml, ".bytesStringArray");
        assertEq(bytesArray[0], hex"01");
        assertEq(bytesArray[1], hex"02");
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

    function test_advancedJsonPath() public {
        bytes memory data = vm.parseToml(toml, ".advancedJsonPath[*].id");
        uint256[] memory numbers = abi.decode(data, (uint256[]));
        assertEq(numbers[0], 1);
        assertEq(numbers[1], 2);
    }

    function test_canonicalizePath() public {
        bytes memory data = vm.parseToml(toml, "$.basicString");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_nonExistentKey() public {
        bytes memory data = vm.parseToml(toml, ".thisKeyDoesNotExist");
        assertEq(0, data.length);
    }

    function test_parseTomlKeys() public {
        string memory tomlString =
            "some_key_to_value = \"some_value\"\n some_key_to_array = [1, 2, 3]\n [some_key_to_object]\n key1 = \"value1\"\n key2 = 2";

        string[] memory keys = vm.parseTomlKeys(tomlString, "$");
        string[] memory expected = new string[](3);
        expected[0] = "some_key_to_value";
        expected[1] = "some_key_to_array";
        expected[2] = "some_key_to_object";
        assertEq(abi.encode(keys), abi.encode(expected));

        keys = vm.parseTomlKeys(tomlString, ".some_key_to_object");
        expected = new string[](2);
        expected[0] = "key1";
        expected[1] = "key2";
        assertEq(abi.encode(keys), abi.encode(expected));

        vm._expectCheatcodeRevert("JSON value at \".some_key_to_array\" is not an object");
        vm.parseTomlKeys(tomlString, ".some_key_to_array");

        vm._expectCheatcodeRevert("JSON value at \".some_key_to_value\" is not an object");
        vm.parseTomlKeys(tomlString, ".some_key_to_value");

        vm._expectCheatcodeRevert("key \".*\" must return exactly one JSON object");
        vm.parseTomlKeys(tomlString, ".*");
    }
}

contract WriteTomlTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    string json1;
    string json2;

    function setUp() public {
        json1 = "example";
        json2 = "example2";
    }

    struct simpleJson {
        uint256 a;
        string b;
    }

    struct notSimpleJson {
        uint256 a;
        string b;
        simpleJson c;
    }

    function test_serializeNotSimpleToml() public {
        string memory json3 = "json3";
        string memory path = "fixtures/Toml/write_complex_test.toml";
        vm.serializeUint(json3, "a", uint256(123));
        string memory semiFinal = vm.serializeString(json3, "b", "test");
        string memory finalJson = vm.serializeString(json3, "c", semiFinal);
        console.log(finalJson);
        vm.writeToml(finalJson, path);
        string memory toml = vm.readFile(path);
        bytes memory data = vm.parseToml(toml);
        notSimpleJson memory decodedData = abi.decode(data, (notSimpleJson));
    }

    function test_retrieveEntireToml() public {
        string memory path = "fixtures/Toml/write_complex_test.toml";
        string memory toml = vm.readFile(path);
        bytes memory data = vm.parseToml(toml, ".");
        notSimpleJson memory decodedData = abi.decode(data, (notSimpleJson));
        console.log(decodedData.a);
        assertEq(decodedData.a, 123);
    }

    function test_checkKeyExists() public {
        string memory path = "fixtures/Toml/write_complex_test.toml";
        string memory toml = vm.readFile(path);
        bool exists = vm.keyExistsToml(toml, ".a");
        assertTrue(exists);
    }

    function test_checkKeyDoesNotExist() public {
        string memory path = "fixtures/Toml/write_complex_test.toml";
        string memory toml = vm.readFile(path);
        bool exists = vm.keyExistsToml(toml, ".d");
        assertTrue(!exists);
    }

    function test_writeToml() public {
        string memory json3 = "json3";
        string memory path = "fixtures/Toml/write_test.toml";
        vm.serializeUint(json3, "a", uint256(123));
        string memory finalJson = vm.serializeString(json3, "b", "test");
        vm.writeToml(finalJson, path);

        string memory toml = vm.readFile(path);
        bytes memory data = vm.parseToml(toml);
        simpleJson memory decodedData = abi.decode(data, (simpleJson));
        assertEq(decodedData.a, 123);
        assertEq(decodedData.b, "test");

        // write json3 to key b
        vm.writeToml(finalJson, path, ".b");
        // read again
        toml = vm.readFile(path);
        data = vm.parseToml(toml, ".b");
        decodedData = abi.decode(data, (simpleJson));
        assertEq(decodedData.a, 123);
        assertEq(decodedData.b, "test");

        // replace a single value to key b
        address ex = address(0xBEEF);
        vm.writeToml(vm.toString(ex), path, ".b");
        toml = vm.readFile(path);
        data = vm.parseToml(toml, ".b");
        address decodedAddress = abi.decode(data, (address));
        assertEq(decodedAddress, ex);
    }
}
