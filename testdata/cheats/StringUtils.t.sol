// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

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
}
