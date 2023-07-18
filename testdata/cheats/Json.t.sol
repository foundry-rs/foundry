// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";
import "../logs/console.sol";

contract ParseJson is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    string json;

    function setUp() public {
        string memory path = "../testdata/fixtures/Json/test.json";
        json = vm.readFile(path);
    }

    function test_uintArray() public {
        bytes memory data = vm.parseJson(json, ".uintArray");
        uint256[] memory decodedData = abi.decode(data, (uint256[]));
        assertEq(42, decodedData[0]);
        assertEq(43, decodedData[1]);
    }

    function test_str() public {
        bytes memory data = vm.parseJson(json, ".str");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_strArray() public {
        bytes memory data = vm.parseJson(json, ".strArray");
        string[] memory decodedData = abi.decode(data, (string[]));
        assertEq("hai", decodedData[0]);
        assertEq("there", decodedData[1]);
    }

    function test_bool() public {
        bytes memory data = vm.parseJson(json, ".bool");
        bool decodedData = abi.decode(data, (bool));
        assertTrue(decodedData);
    }

    function test_boolArray() public {
        bytes memory data = vm.parseJson(json, ".boolArray");
        bool[] memory decodedData = abi.decode(data, (bool[]));
        assertTrue(decodedData[0]);
        assertTrue(!decodedData[1]);
    }

    function test_address() public {
        bytes memory data = vm.parseJson(json, ".address");
        address decodedData = abi.decode(data, (address));
        assertEq(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, decodedData);
    }

    function test_addressArray() public {
        bytes memory data = vm.parseJson(json, ".addressArray");
        address[] memory decodedData = abi.decode(data, (address[]));
        assertEq(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, decodedData[0]);
        assertEq(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, decodedData[1]);
    }

    function test_H160ButNotaddress() public {
        string memory data = abi.decode(vm.parseJson(json, ".H160NotAddress"), (string));
        assertEq("0000000000000000000000000000000000001337", data);
    }

    struct Nested {
        uint256 number;
        string str;
    }

    function test_nestedObject() public {
        bytes memory data = vm.parseJson(json, ".nestedObject");
        Nested memory nested = abi.decode(data, (Nested));
        assertEq(nested.number, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
        assertEq(nested.str, "NEST");
    }

    struct Whole {
        string str;
        string[] strArray;
        uint256[] uintArray;
    }

    function test_wholeObject() public {
        // we need to make the path relative to the crate that's running tests for it (forge crate)
        string memory path = "../testdata/fixtures/Json/wholeJson.json";
        console.log(path);
        json = vm.readFile(path);
        bytes memory data = vm.parseJson(json);
        Whole memory whole = abi.decode(data, (Whole));
        assertEq(whole.str, "hai");
        assertEq(whole.uintArray[0], 42);
        assertEq(whole.uintArray[1], 43);
        assertEq(whole.strArray[0], "hai");
        assertEq(whole.strArray[1], "there");
    }

    function test_coercionRevert() public {
        vm.expectRevert(
            "You can only coerce values or arrays, not JSON objects. The key '.nestedObject' returns an object"
        );
        uint256 number = this.parseJsonUint(json, ".nestedObject");
    }

    function parseJsonUint(string memory json, string memory path) public returns (uint256) {
        uint256 data = vm.parseJsonUint(json, path);
    }

    function test_coercionUint() public {
        uint256 number = vm.parseJsonUint(json, ".hexUint");
        assertEq(number, 1231232);
        number = vm.parseJsonUint(json, ".stringUint");
        assertEq(number, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
        number = vm.parseJsonUint(json, ".numberUint");
        assertEq(number, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
        uint256[] memory numbers = vm.parseJsonUintArray(json, ".arrayUint");
        assertEq(numbers[0], 1231232);
        assertEq(numbers[1], 1231232);
        assertEq(numbers[2], 1231232);
    }

    function test_coercionInt() public {
        int256 number = vm.parseJsonInt(json, ".hexInt");
        assertEq(number, -12);
        number = vm.parseJsonInt(json, ".stringInt");
        assertEq(number, -12);
    }

    function test_coercionBool() public {
        bool boolean = vm.parseJsonBool(json, ".booleanString");
        assertEq(boolean, true);
        bool[] memory booleans = vm.parseJsonBoolArray(json, ".booleanArray");
        assert(booleans[0]);
        assert(!booleans[1]);
    }

    function test_advancedJsonPath() public {
        bytes memory data = vm.parseJson(json, ".advancedJsonPath[*].id");
        uint256[] memory numbers = abi.decode(data, (uint256[]));
        assertEq(numbers[0], 1);
        assertEq(numbers[1], 2);
    }

    function test_canonicalizePath() public {
        bytes memory data = vm.parseJson(json, "$.str");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }
}

contract WriteJson is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    string json1;
    string json2;

    function setUp() public {
        json1 = "example";
        json2 = "example2";
    }

    function test_serializeJson() public {
        vm.serializeUint(json1, "key1", uint256(254));
        vm.serializeBool(json1, "boolean", true);
        vm.serializeInt(json2, "key2", -234);
        vm.serializeUint(json2, "deploy", uint256(254));
        string memory data = vm.serializeBool(json2, "boolean", true);
        vm.serializeString(json2, "json1", data);
        emit log(data);
    }

    function test_serializeArray() public {
        bool[] memory data1 = new bool[](3);
        data1[0] = true;
        data1[2] = false;
        vm.serializeBool(json1, "array1", data1);

        address[] memory data2 = new address[](3);
        data2[0] = address(0xBEEEF);
        data2[2] = vm.addr(123);
        vm.serializeAddress(json1, "array2", data2);

        bytes[] memory data3 = new bytes[](3);
        data3[0] = bytes("123");
        data3[2] = bytes("fpovhpgjaiosfjhapiufpsdf");
        string memory finalJson = vm.serializeBytes(json1, "array3", data3);

        string memory path = "../testdata/fixtures/Json/write_test_array.json";
        vm.writeJson(finalJson, path);

        string memory json = vm.readFile(path);
        bytes memory rawData = vm.parseJson(json, ".array1");
        bool[] memory parsedData1 = new bool[](3);
        parsedData1 = abi.decode(rawData, (bool[]));
        assertEq(parsedData1[0], data1[0]);
        assertEq(parsedData1[1], data1[1]);
        assertEq(parsedData1[2], data1[2]);

        rawData = vm.parseJson(json, ".array2");
        address[] memory parsedData2 = new address[](3);
        parsedData2 = abi.decode(rawData, (address[]));
        assertEq(parsedData2[0], data2[0]);
        assertEq(parsedData2[1], data2[1]);
        assertEq(parsedData2[2], data2[2]);

        rawData = vm.parseJson(json, ".array3");
        bytes[] memory parsedData3 = new bytes[](3);
        parsedData3 = abi.decode(rawData, (bytes[]));
        assertEq(parsedData3[0], data3[0]);
        assertEq(parsedData3[1], data3[1]);
        assertEq(parsedData3[2], data3[2]);
        vm.removeFile(path);
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

    function test_serializeNotSimpleJson() public {
        string memory json3 = "json3";
        string memory path = "../testdata/fixtures/Json/write_complex_test.json";
        vm.serializeUint(json3, "a", uint256(123));
        string memory semiFinal = vm.serializeString(json3, "b", "test");
        string memory finalJson = vm.serializeString(json3, "c", semiFinal);
        console.log(finalJson);
        vm.writeJson(finalJson, path);
        string memory json = vm.readFile(path);
        bytes memory data = vm.parseJson(json);
        notSimpleJson memory decodedData = abi.decode(data, (notSimpleJson));
    }

    function test_retrieveEntireJson() public {
        string memory path = "../testdata/fixtures/Json/write_complex_test.json";
        string memory json = vm.readFile(path);
        bytes memory data = vm.parseJson(json, ".");
        notSimpleJson memory decodedData = abi.decode(data, (notSimpleJson));
        console.log(decodedData.a);
        assertEq(decodedData.a, 12345);
    }

    function test_checkKeyExists() public {
        string memory path = "../testdata/fixtures/Json/write_complex_test.json";
        string memory json = vm.readFile(path);
        bool exists = vm.keyExists(json, "a");
        assert(exists);
    }

    function test_writeJson() public {
        string memory json3 = "json3";
        string memory path = "../testdata/fixtures/Json/write_test.json";
        vm.serializeUint(json3, "a", uint256(123));
        string memory finalJson = vm.serializeString(json3, "b", "test");
        vm.writeJson(finalJson, path);

        string memory json = vm.readFile(path);
        bytes memory data = vm.parseJson(json);
        simpleJson memory decodedData = abi.decode(data, (simpleJson));
        assertEq(decodedData.a, 123);
        assertEq(decodedData.b, "test");

        // write json3 to key b
        vm.writeJson(finalJson, path, ".b");
        // read again
        json = vm.readFile(path);
        data = vm.parseJson(json, ".b");
        decodedData = abi.decode(data, (simpleJson));
        assertEq(decodedData.a, 123);
        assertEq(decodedData.b, "test");

        // replace a single value to key b
        address ex = address(0xBEEF);
        vm.writeJson(vm.toString(ex), path, ".b");
        json = vm.readFile(path);
        data = vm.parseJson(json, ".b");
        address decodedAddress = abi.decode(data, (address));
        assertEq(decodedAddress, ex);
    }
}
