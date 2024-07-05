// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";
import "../logs/console.sol";

library JsonStructs {
    address constant HEVM_ADDRESS = address(bytes20(uint160(uint256(keccak256("hevm cheat code")))));
    Vm constant vm = Vm(HEVM_ADDRESS);

    // forge eip712 testdata/default/cheats/Json.t.sol -R 'cheats=testdata/cheats' -R 'ds-test=testdata/lib/ds-test/src' | grep ^FlatJson
    string constant schema_FlatJson =
        "FlatJson(uint256 a,int24[][] arr,string str,bytes b,address addr,bytes32 fixedBytes)";

    // forge eip712 testdata/default/cheats/Json.t.sol -R 'cheats=testdata/cheats' -R 'ds-test=testdata/lib/ds-test/src' | grep ^NestedJson
    string constant schema_NestedJson =
        "NestedJson(FlatJson[] members,AnotherFlatJson inner,string name)AnotherFlatJson(bytes4 fixedBytes)FlatJson(uint256 a,int24[][] arr,string str,bytes b,address addr,bytes32 fixedBytes)";

    function deserializeFlatJson(string memory json) internal pure returns (ParseJsonTest.FlatJson memory) {
        return abi.decode(vm.parseJsonType(json, schema_FlatJson), (ParseJsonTest.FlatJson));
    }

    function deserializeFlatJson(string memory json, string memory path)
        internal
        pure
        returns (ParseJsonTest.FlatJson memory)
    {
        return abi.decode(vm.parseJsonType(json, path, schema_FlatJson), (ParseJsonTest.FlatJson));
    }

    function deserializeFlatJsonArray(string memory json, string memory path)
        internal
        pure
        returns (ParseJsonTest.FlatJson[] memory)
    {
        return abi.decode(vm.parseJsonTypeArray(json, path, schema_FlatJson), (ParseJsonTest.FlatJson[]));
    }

    function deserializeNestedJson(string memory json) internal pure returns (ParseJsonTest.NestedJson memory) {
        return abi.decode(vm.parseJsonType(json, schema_NestedJson), (ParseJsonTest.NestedJson));
    }

    function deserializeNestedJson(string memory json, string memory path)
        internal
        pure
        returns (ParseJsonTest.NestedJson memory)
    {
        return abi.decode(vm.parseJsonType(json, path, schema_NestedJson), (ParseJsonTest.NestedJson));
    }

    function deserializeNestedJsonArray(string memory json, string memory path)
        internal
        pure
        returns (ParseJsonTest.NestedJson[] memory)
    {
        return abi.decode(vm.parseJsonType(json, path, schema_NestedJson), (ParseJsonTest.NestedJson[]));
    }

    function serialize(ParseJsonTest.FlatJson memory instance) internal pure returns (string memory) {
        return vm.serializeJsonType(schema_FlatJson, abi.encode(instance));
    }

    function serialize(ParseJsonTest.NestedJson memory instance) internal pure returns (string memory) {
        return vm.serializeJsonType(schema_NestedJson, abi.encode(instance));
    }
}

contract ParseJsonTest is DSTest {
    using JsonStructs for *;

    struct FlatJson {
        uint256 a;
        int24[][] arr;
        string str;
        bytes b;
        address addr;
        bytes32 fixedBytes;
    }

    struct AnotherFlatJson {
        bytes4 fixedBytes;
    }

    struct NestedJson {
        FlatJson[] members;
        AnotherFlatJson inner;
        string name;
    }

    Vm constant vm = Vm(HEVM_ADDRESS);
    string json;

    function setUp() public {
        string memory path = "fixtures/Json/test.json";
        json = vm.readFile(path);
    }

    function test_basicString() public {
        bytes memory data = vm.parseJson(json, ".basicString");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_null() public {
        bytes memory data = vm.parseJson(json, ".null");
        bytes memory decodedData = abi.decode(data, (bytes));
        assertEq(new bytes(0), decodedData);
    }

    function test_stringArray() public {
        bytes memory data = vm.parseJson(json, ".stringArray");
        string[] memory decodedData = abi.decode(data, (string[]));
        assertEq("hai", decodedData[0]);
        assertEq("there", decodedData[1]);
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

    function test_bool() public {
        bytes memory data = vm.parseJson(json, ".boolTrue");
        bool decodedData = abi.decode(data, (bool));
        assertTrue(decodedData);

        data = vm.parseJson(json, ".boolFalse");
        decodedData = abi.decode(data, (bool));
        assertTrue(!decodedData);
    }

    function test_boolArray() public {
        bytes memory data = vm.parseJson(json, ".boolArray");
        bool[] memory decodedData = abi.decode(data, (bool[]));
        assertTrue(decodedData[0]);
        assertTrue(!decodedData[1]);
    }

    function test_uintArray() public {
        bytes memory data = vm.parseJson(json, ".uintArray");
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

    function test_wholeObject() public {
        // we need to make the path relative to the crate that's running tests for it (forge crate)
        string memory path = "fixtures/Json/whole_json.json";
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
        vm._expectCheatcodeRevert("expected uint256, found JSON object");
        vm.parseJsonUint(json, ".nestedObject");
    }

    function test_coercionUint() public {
        uint256 number = vm.parseJsonUint(json, ".uintHex");
        assertEq(number, 1231232);
        number = vm.parseJsonUint(json, ".uintString");
        assertEq(number, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
        number = vm.parseJsonUint(json, ".uintNumber");
        assertEq(number, 115792089237316195423570985008687907853269984665640564039457584007913129639935);
        uint256[] memory numbers = vm.parseJsonUintArray(json, ".uintArray");
        assertEq(numbers[0], 42);
        assertEq(numbers[1], 43);
        numbers = vm.parseJsonUintArray(json, ".uintStringArray");
        assertEq(numbers[0], 1231232);
        assertEq(numbers[1], 1231232);
        assertEq(numbers[2], 1231232);
    }

    function test_coercionInt() public {
        int256 number = vm.parseJsonInt(json, ".intNumber");
        assertEq(number, -12);
        number = vm.parseJsonInt(json, ".intString");
        assertEq(number, -12);
        number = vm.parseJsonInt(json, ".intHex");
        assertEq(number, -12);
    }

    function test_coercionBool() public {
        bool boolean = vm.parseJsonBool(json, ".boolTrue");
        assertTrue(boolean);
        bool boolFalse = vm.parseJsonBool(json, ".boolFalse");
        assertTrue(!boolFalse);
        boolean = vm.parseJsonBool(json, ".boolString");
        assertEq(boolean, true);
        bool[] memory booleans = vm.parseJsonBoolArray(json, ".boolArray");
        assertTrue(booleans[0]);
        assertTrue(!booleans[1]);
        booleans = vm.parseJsonBoolArray(json, ".boolStringArray");
        assertTrue(booleans[0]);
        assertTrue(!booleans[1]);
    }

    function test_coercionBytes() public {
        bytes memory bytes_ = vm.parseJsonBytes(json, ".bytesString");
        assertEq(bytes_, hex"01");

        bytes[] memory bytesArray = vm.parseJsonBytesArray(json, ".bytesStringArray");
        assertEq(bytesArray[0], hex"01");
        assertEq(bytesArray[1], hex"02");
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

    function test_advancedJsonPath() public {
        bytes memory data = vm.parseJson(json, ".advancedJsonPath[*].id");
        uint256[] memory numbers = abi.decode(data, (uint256[]));
        assertEq(numbers[0], 1);
        assertEq(numbers[1], 2);
    }

    function test_canonicalizePath() public {
        bytes memory data = vm.parseJson(json, "$.basicString");
        string memory decodedData = abi.decode(data, (string));
        assertEq("hai", decodedData);
    }

    function test_nonExistentKey() public {
        bytes memory data = vm.parseJson(json, ".thisKeyDoesNotExist");
        assertEq(0, data.length);
    }

    function test_parseJsonKeys() public {
        string memory jsonString =
            '{"some_key_to_value": "some_value", "some_key_to_array": [1, 2, 3], "some_key_to_object": {"key1": "value1", "key2": 2}}';

        string[] memory keys = vm.parseJsonKeys(jsonString, "$");
        string[] memory expected = new string[](3);
        expected[0] = "some_key_to_value";
        expected[1] = "some_key_to_array";
        expected[2] = "some_key_to_object";
        assertEq(abi.encode(keys), abi.encode(expected));

        keys = vm.parseJsonKeys(jsonString, ".some_key_to_object");
        expected = new string[](2);
        expected[0] = "key1";
        expected[1] = "key2";
        assertEq(abi.encode(keys), abi.encode(expected));

        vm._expectCheatcodeRevert("JSON value at \".some_key_to_array\" is not an object");
        vm.parseJsonKeys(jsonString, ".some_key_to_array");

        vm._expectCheatcodeRevert("JSON value at \".some_key_to_value\" is not an object");
        vm.parseJsonKeys(jsonString, ".some_key_to_value");

        vm._expectCheatcodeRevert("key \".*\" must return exactly one JSON object");
        vm.parseJsonKeys(jsonString, ".*");
    }

    // forge eip712 testdata/default/cheats/Json.t.sol -R 'cheats=testdata/cheats' -R 'ds-test=testdata/lib/ds-test/src' | grep ^FlatJson
    string constant schema_FlatJson =
        "FlatJson(uint256 a,int24[][] arr,string str,bytes b,address addr,bytes32 fixedBytes)";

    // forge eip712 testdata/default/cheats/Json.t.sol -R 'cheats=testdata/cheats' -R 'ds-test=testdata/lib/ds-test/src' | grep ^NestedJson
    string constant schema_NestedJson =
        "NestedJson(FlatJson[] members,AnotherFlatJson inner,string name)AnotherFlatJson(bytes4 fixedBytes)FlatJson(uint256 a,int24[][] arr,string str,bytes b,address addr,bytes32 fixedBytes)";

    function test_parseJsonType() public {
        string memory readJson = vm.readFile("fixtures/Json/nested_json_struct.json");
        NestedJson memory data = readJson.deserializeNestedJson();
        assertEq(data.members.length, 2);

        FlatJson memory expected = FlatJson({
            a: 200,
            arr: new int24[][](0),
            str: "some other string",
            b: hex"0000000000000000000000000000000000000000",
            addr: 0x167D91deaEEE3021161502873d3bcc6291081648,
            fixedBytes: 0xed1c7beb1f00feaaaec5636950d6edb25a8d4fedc8deb2711287b64c4d27719d
        });

        assertEq(keccak256(abi.encode(data.members[1])), keccak256(abi.encode(expected)));
        assertEq(bytes32(data.inner.fixedBytes), bytes32(bytes4(0x12345678)));

        FlatJson[] memory members = JsonStructs.deserializeFlatJsonArray(readJson, ".members");

        assertEq(keccak256(abi.encode(members)), keccak256(abi.encode(data.members)));
    }

    function test_parseJsonType_roundtrip() public {
        string memory readJson = vm.readFile("fixtures/Json/nested_json_struct.json");
        NestedJson memory data = readJson.deserializeNestedJson();
        string memory serialized = data.serialize();
        NestedJson memory deserialized = serialized.deserializeNestedJson();
        assertEq(keccak256(abi.encode(data)), keccak256(abi.encode(deserialized)));
    }
}

contract WriteJsonTest is DSTest {
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
        vm.serializeUintToHex(json2, "hexUint", uint256(255));
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

        string memory path = "fixtures/Json/write_test_array.json";
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

    // The serializeJson cheatcode was added to support assigning an existing json string to an object key.
    // Github issue: https://github.com/foundry-rs/foundry/issues/5745
    function test_serializeRootObject() public {
        string memory serialized = vm.serializeJson(json1, '{"foo": "bar"}');
        assertEq(serialized, '{"foo": "bar"}');
        serialized = vm.serializeBool(json1, "boolean", true);
        assertEq(vm.parseJsonString(serialized, ".foo"), "bar");
        assertEq(vm.parseJsonBool(serialized, ".boolean"), true);

        string memory overwritten = vm.serializeJson(json1, '{"value": 123}');
        assertEq(overwritten, '{"value": 123}');
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
        string memory path = "fixtures/Json/write_complex_test.json";
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
        string memory path = "fixtures/Json/write_complex_test.json";
        string memory json = vm.readFile(path);
        bytes memory data = vm.parseJson(json, ".");
        notSimpleJson memory decodedData = abi.decode(data, (notSimpleJson));
        console.log(decodedData.a);
        assertEq(decodedData.a, 123);
    }

    function test_checkKeyExistsJson() public {
        string memory path = "fixtures/Json/write_complex_test.json";
        string memory json = vm.readFile(path);
        bool exists = vm.keyExistsJson(json, ".a");
        assertTrue(exists);

        // TODO: issue deprecation warning
        exists = vm.keyExists(json, ".a");
        assertTrue(exists);
    }

    function test_checkKeyDoesNotExistJson() public {
        string memory path = "fixtures/Json/write_complex_test.json";
        string memory json = vm.readFile(path);
        bool exists = vm.keyExistsJson(json, ".d");
        assertTrue(!exists);

        // TODO: issue deprecation warning
        exists = vm.keyExists(json, ".d");
        assertTrue(!exists);
    }

    function test_writeJson() public {
        string memory json3 = "json3";
        string memory path = "fixtures/Json/write_test.json";
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
