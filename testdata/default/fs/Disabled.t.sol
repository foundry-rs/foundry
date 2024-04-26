// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract DisabledTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testReadFile() public {
        string memory path = "fixtures/File/read.txt";
        vm._expectCheatcodeRevert();
        vm.readFile(path);
    }

    function testReadLine() public {
        string memory path = "fixtures/File/read.txt";
        vm._expectCheatcodeRevert();
        vm.readLine(path);
    }

    function testWriteFile() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm._expectCheatcodeRevert();
        vm.writeFile(path, data);
    }

    function testWriteLine() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm._expectCheatcodeRevert();
        vm.writeLine(path, data);
    }

    function testRemoveFile() public {
        string memory path = "fixtures/File/write_file.txt";
        vm._expectCheatcodeRevert();
        vm.removeFile(path);
    }
}
