// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";
import "../logs/console.sol";

contract ParseJson is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    string json;

    function setUp() public {
        string memory path = "../testdata/fixtures/Json/test.json";
        json = cheats.readFile(path);
    }

    function test_uintArray() public {
        bytes memory data = cheats.parseJson(json, ".uintArray");
        uint256[] memory decodedData = abi.decode(data, (uint256[]));
        assertEq(42, decodedData[0]);
        assertEq(43, decodedData[1]);
    }

    function test_str() public {
        bytes memory data = cheats.parseJson(json, ".str");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_strArray() public {
        bytes memory data = cheats.parseJson(json, ".strArray");
        string[] memory decodedData = abi.decode(data, (string[]));
        assertEq("hai", decodedData[0]);
        assertEq("there", decodedData[1]);
    }

    function test_bool() public {
        bytes memory data = cheats.parseJson(json, ".bool");
        bool decodedData = abi.decode(data, (bool));
        assertTrue(decodedData);
    }

    function test_boolArray() public {
        bytes memory data = cheats.parseJson(json, ".boolArray");
        bool[] memory decodedData = abi.decode(data, (bool[]));
        assertTrue(decodedData[0]);
        assertTrue(!decodedData[1]);
    }

    function test_address() public {
        bytes memory data = cheats.parseJson(json, ".address");
        address decodedData = abi.decode(data, (address));
        assertEq(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, decodedData);
    }

    function test_addressArray() public {
        bytes memory data = cheats.parseJson(json, ".addressArray");
        address[] memory decodedData = abi.decode(data, (address[]));
        assertEq(0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266, decodedData[0]);
        assertEq(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D, decodedData[1]);
    }

    function test_H160ButNotaddress() public {
        string memory data = abi.decode(cheats.parseJson(json, ".H160NotAddress"), (string));
        assertEq("0000000000000000000000000000000000001337", data);
    }

    struct Nested {
        uint256 number;
        string str;
    }

    function test_nestedObject() public {
        bytes memory data = cheats.parseJson(json, ".nestedObject");
        Nested memory nested = abi.decode(data, (Nested));
        assertEq(nested.number, 13);
        assertEq(nested.str, "NEST");
    }

    struct Whole {
        string str;
        string[] strArray;
        uint256[] uintArray;
    }

    function test_wholeObject() public {
        // we need to make the path relative to the crate that's running tests for it (forge crate)
        string memory root = cheats.envString("CARGO_MANIFEST_DIR");
        string memory path = string.concat(root, "/../testdata/fixtures/Json/wholeJson.json");
        console.log(path);
        json = cheats.readFile(path);
        bytes memory data = cheats.parseJson(json);
        Whole memory whole = abi.decode(data, (Whole));
        assertEq(whole.str, "hai");
        assertEq(whole.uintArray[0], 42);
        assertEq(whole.uintArray[1], 43);
        assertEq(whole.strArray[0], "hai");
        assertEq(whole.strArray[1], "there");
    }
}

contract WriteJson is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

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
        vm.serializeBytes(json1, "array3", data3);

        string memory path = "../testdata/fixtures/Json/write_test_array.json";
        vm.writeJson(json1, path);

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

    function test_writeJson() public {
        string memory json3 = "json3";
        string memory path = "../testdata/fixtures/Json/write_test.json";
        vm.serializeUint(json3, "a", uint256(123));
        vm.serializeString(json3, "b", "test");
        vm.writeJson(json3, path);

        string memory json = vm.readFile(path);
        bytes memory data = vm.parseJson(json);
        simpleJson memory decodedData = abi.decode(data, (simpleJson));
        assertEq(decodedData.a, 123);
        assertEq(decodedData.b, "test");

        // write json3 to key b
        vm.writeJson(json3, path, ".b");
        // read again
        json = vm.readFile(path);
        data = vm.parseJson(json, ".b");
        decodedData = abi.decode(data, (simpleJson));
        assertEq(decodedData.a, 123);
        assertEq(decodedData.b, "test");
    }
}
