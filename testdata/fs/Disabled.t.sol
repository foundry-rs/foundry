// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

contract DisabledTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    bytes constant FOUNDRY_READ_ERR =
        "the path fixtures/File/read.txt is not allowed to be accessed for read operations";
    bytes constant FOUNDRY_WRITE_ERR =
        "the path fixtures/File/write_file.txt is not allowed to be accessed for write operations";

    function testReadFile() public {
        string memory path = "fixtures/File/read.txt";
        vm.expectRevert(FOUNDRY_READ_ERR);
        vm.readFile(path);
    }

    function testReadLine() public {
        string memory path = "fixtures/File/read.txt";
        vm.expectRevert(FOUNDRY_READ_ERR);
        vm.readLine(path);
    }

    function testWriteFile() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.writeFile(path, data);
    }

    function testWriteLine() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.writeLine(path, data);
    }

    function testRemoveFile() public {
        string memory path = "fixtures/File/write_file.txt";
        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.removeFile(path);
    }
}
