// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract StringManipulationTest is Test {
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
}
