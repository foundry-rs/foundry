//@compile-flags: --severity info

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {Test} from "./auxiliary/Test.sol";

contract UnsafeCheatcodes is Test {
    function testSafeCheatcodes() public {
        vm.prank(address(0x1));
        vm.deal(address(0x1), 1 ether);
        vm.warp(block.timestamp + 1);
        vm.roll(block.number + 1);
        vm.assume(true);
        vm.expectRevert();
    }

    function testDirectFfi() public {
        string[] memory inputs = new string[](1);
        vm.ffi(inputs); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectReadFile() public {
        vm.readFile("test.txt"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectReadLine() public {
        vm.readLine("test.txt"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectWriteFile() public {
        vm.writeFile("test.txt", "data"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectWriteLine() public {
        vm.writeLine("test.txt", "data"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectRemoveFile() public {
        vm.removeFile("test.txt"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectCloseFile() public {
        vm.closeFile("test.txt"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectSetEnv() public {
        vm.setEnv("KEY", "value"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testDirectDeriveKey() public {
        vm.deriveKey("mnemonic", 0); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testAssignmentFfi() public {
        string[] memory inputs = new string[](1);
        bytes memory result = vm.ffi(inputs); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

    function testMultipleUnsafe() public {
        vm.ffi(new string[](1)); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
        vm.setEnv("KEY", "value"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
        vm.readFile("test.txt"); //~NOTE: usage of unsafe cheatcodes that can perform dangerous operations
    }

}
