// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract ParseJson is DSTest {

    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    string json;
    function setUp() public {
        string memory path = "../testdata/fixtures/ParseJson/test.json";
        json = cheats.readFile(path);
    }

    function test_uintArray() public {
        bytes memory data = cheats.parseJson(json, ".uintArray");
        uint[] memory decodedData = abi.decode(data, (uint[]));
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

    struct Nested{
        uint256 number;
        string str;
    }

    function test_nestedObject() public {
        bytes memory data = cheats.parseJson(json, ".nestedObject");
        Nested memory nested = abi.decode(data, (Nested));
        assertEq(nested.number,13);
        assertEq(nested.str,"NEST");
    }

}
