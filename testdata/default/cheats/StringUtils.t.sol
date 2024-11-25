// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract StringManipulationTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testToLowercase() public {
        string memory original = "Hello World";
        string memory lowercased = vm.toLowercase(original);
        assertEq("hello world", lowercased);
    }

    function testToUppercase() public {
        string memory original = "Hello World";
        string memory uppercased = vm.toUppercase(original);
        assertEq("HELLO WORLD", uppercased);
    }

    function testTrim() public {
        string memory original = "   Hello World   ";
        string memory trimmed = vm.trim(original);
        assertEq("Hello World", trimmed);
    }

    function testReplace() public {
        string memory original = "Hello World";
        string memory replaced = vm.replace(original, "World", "Reth");
        assertEq("Hello Reth", replaced);
    }

    function testSplit() public {
        string memory original = "Hello,World,Reth";
        string[] memory splitResult = vm.split(original, ",");
        assertEq(3, splitResult.length);
        assertEq("Hello", splitResult[0]);
        assertEq("World", splitResult[1]);
        assertEq("Reth", splitResult[2]);
    }

    function testIndexOf() public {
        string memory input = "Hello, World!";
        string memory key1 = "Hello,";
        string memory key2 = "World!";
        string memory key3 = "";
        string memory key4 = "foundry";
        assertEq(vm.indexOf(input, key1), 0);
        assertEq(vm.indexOf(input, key2), 7);
        assertEq(vm.indexOf(input, key3), 0);
        assertEq(vm.indexOf(input, key4), type(uint256).max);
    }

    function testContains() public {
        string memory subject = "this is a test";
        assert(vm.contains(subject, "test"));
        assert(!vm.contains(subject, "foundry"));
    }

    function testFormat() public {
        string memory input1 = "test: %d %d %d %d";
        assertEq(vm.format(input1, 1), "test: 1 %d %d %d");
        assertEq(vm.format(input1, 1, 2), "test: 1 2 %d %d");
        assertEq(vm.format(input1, 1, 2, 3), "test: 1 2 3 %d");
        assertEq(vm.format(input1, 1, 2, 3, 4), "test: 1 2 3 4");

        string memory input2 = "test: %s %s %s %s";
        assertEq(vm.format(input2, true), "test: true %s %s %s");
        assertEq(vm.format(input2, true, false), "test: true false %s %s");
        assertEq(vm.format(input2, true, false, true), "test: true false true %s");
        assertEq(vm.format(input2, true, false, true, true), "test: true false true true");
    
        assertEq(vm.format("test: %d %s", 1, true), "test: 1 true");
        assertEq(vm.format("test: %s %d", true, 1), "test: true 1");
    }
}
