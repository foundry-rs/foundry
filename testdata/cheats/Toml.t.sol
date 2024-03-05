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
