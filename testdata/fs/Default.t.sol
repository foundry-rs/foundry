// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

contract DefaultAccessTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testReadFile() public {
        string memory path = "fixtures/File/read.txt";
        vm.readFile(path);

        vm.readFileBinary(path);
    }

    function testReadLine() public {
        string memory path = "fixtures/File/read.txt";
        vm.readLine(path);
    }

    function testWriteFile() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";

        vm.expectRevert();
        vm.writeFile(path, data);

        vm.expectRevert();
        vm.writeFileBinary(path, bytes(data));
    }

    function testWriteLine() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";

        vm.expectRevert();
        vm.writeLine(path, data);
    }

    function testRemoveFile() public {
        string memory path = "fixtures/File/write_file.txt";

        vm.expectRevert();
        vm.removeFile(path);
    }
}
