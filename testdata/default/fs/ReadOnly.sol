// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// Default permissions: only read FS operations should succeed.

/// forge-config: default.fs_permissions = [{ access = "read", path = "./fixtures"}]
contract ReadOnlyAccessTest is Test {
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
        string memory path = "fixtures/File/ignored/write_file.txt";
        string memory data = "hello writable world";

        vm._expectCheatcodeRevert();
        vm.writeFile(path, data);

        vm._expectCheatcodeRevert();
        vm.writeFileBinary(path, bytes(data));
    }

    function testWriteLine() public {
        string memory path = "fixtures/File/ignored/write_file.txt";
        string memory data = "hello writable world";

        vm._expectCheatcodeRevert();
        vm.writeLine(path, data);
    }

    function testRemoveFile() public {
        string memory path = "fixtures/File/ignored/write_file.txt";

        vm._expectCheatcodeRevert();
        vm.removeFile(path);
    }
}
