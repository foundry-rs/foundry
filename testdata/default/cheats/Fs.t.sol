// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract FsTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    bytes constant FOUNDRY_TOML_ACCESS_ERR = "access to foundry.toml is not allowed";
    bytes constant FOUNDRY_READ_ERR = "the path /etc/hosts is not allowed to be accessed for read operations";
    bytes constant FOUNDRY_READ_DIR_ERR = "the path /etc is not allowed to be accessed for read operations";
    bytes constant FOUNDRY_WRITE_ERR = "the path /etc/hosts is not allowed to be accessed for write operations";

    function assertEntry(Vm.DirEntry memory entry, uint64 depth, bool dir) private {
        assertEq(entry.errorMessage, "");
        assertEq(entry.depth, depth);
        assertEq(entry.isDir, dir);
        assertEq(entry.isSymlink, false);
    }

    function testReadFile() public {
        string memory path = "fixtures/File/read.txt";

        assertEq(vm.readFile(path), "hello readable world\nthis is the second line!");

        vm._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        vm.readFile("/etc/hosts");

        vm._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        vm.readFileBinary("/etc/hosts");
    }

    function testReadLine() public {
        string memory path = "fixtures/File/read.txt";

        assertEq(vm.readLine(path), "hello readable world");
        assertEq(vm.readLine(path), "this is the second line!");
        assertEq(vm.readLine(path), "");

        vm._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        vm.readLine("/etc/hosts");
    }

    function testWriteFile() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm.writeFile(path, data);

        assertEq(vm.readFile(path), data);

        vm.removeFile(path);

        vm._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        vm.writeFile("/etc/hosts", "malicious stuff");
        vm._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        vm.writeFileBinary("/etc/hosts", "malicious stuff");
    }

    function testCopyFile() public {
        string memory from = "fixtures/File/read.txt";
        string memory to = "fixtures/File/copy.txt";
        uint64 copied = vm.copyFile(from, to);
        assertEq(vm.fsMetadata(to).length, uint256(copied));
        assertEq(vm.readFile(from), vm.readFile(to));
        vm.removeFile(to);
    }

    function testWriteLine() public {
        string memory path = "fixtures/File/write_line.txt";

        string memory line1 = "first line";
        vm.writeLine(path, line1);

        string memory line2 = "second line";
        vm.writeLine(path, line2);

        assertEq(vm.readFile(path), string.concat(line1, "\n", line2, "\n"));

        vm.removeFile(path);

        vm._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        vm.writeLine("/etc/hosts", "malicious stuff");
    }

    function testCloseFile() public {
        string memory path = "fixtures/File/read.txt";

        assertEq(vm.readLine(path), "hello readable world");
        vm.closeFile(path);
        assertEq(vm.readLine(path), "hello readable world");
    }

    function testRemoveFile() public {
        string memory path = "fixtures/File/remove_file.txt";
        string memory data = "hello writable world";

        vm.writeFile(path, data);
        assertEq(vm.readLine(path), data);

        vm.removeFile(path);
        vm.writeLine(path, data);
        assertEq(vm.readLine(path), data);

        vm.removeFile(path);

        vm._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        vm.removeFile("/etc/hosts");
    }

    function testWriteLineFoundrytoml() public {
        string memory root = vm.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        vm._expectCheatcodeRevert();
        vm.writeLine(foundryToml, "\nffi = true\n");

        vm._expectCheatcodeRevert();
        vm.writeLine("foundry.toml", "\nffi = true\n");

        vm._expectCheatcodeRevert();
        vm.writeLine("./foundry.toml", "\nffi = true\n");

        vm._expectCheatcodeRevert();
        vm.writeLine("./Foundry.toml", "\nffi = true\n");
    }

    function testWriteFoundrytoml() public {
        string memory root = vm.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        vm._expectCheatcodeRevert();
        vm.writeFile(foundryToml, "\nffi = true\n");

        vm._expectCheatcodeRevert();
        vm.writeFile("foundry.toml", "\nffi = true\n");

        vm._expectCheatcodeRevert();
        vm.writeFile("./foundry.toml", "\nffi = true\n");

        vm._expectCheatcodeRevert();
        vm.writeFile("./Foundry.toml", "\nffi = true\n");
    }

    function testReadDir() public {
        string memory path = "fixtures/Dir";

        {
            Vm.DirEntry[] memory entries = vm.readDir(path);
            assertEq(entries.length, 2);
            assertEntry(entries[0], 1, false);
            assertEntry(entries[1], 1, true);

            Vm.DirEntry[] memory entries2 = vm.readDir(path, 1);
            assertEq(entries2.length, 2);
            assertEq(entries[0].path, entries2[0].path);
            assertEq(entries[1].path, entries2[1].path);

            string memory contents = vm.readFile(entries[0].path);
            assertEq(contents, unicode"Wow! 😀");
        }

        {
            Vm.DirEntry[] memory entries = vm.readDir(path, 2);
            assertEq(entries.length, 4);
            assertEntry(entries[2], 2, false);
            assertEntry(entries[3], 2, true);
        }

        {
            Vm.DirEntry[] memory entries = vm.readDir(path, 3);
            assertEq(entries.length, 5);
            assertEntry(entries[4], 3, false);
        }

        vm._expectCheatcodeRevert(FOUNDRY_READ_DIR_ERR);
        vm.readDir("/etc");
    }

    function testCreateRemoveDir() public {
        string memory path = "fixtures/Dir/remove_dir";
        string memory child = string.concat(path, "/child");

        vm.createDir(path, false);
        assertEq(vm.fsMetadata(path).isDir, true);

        vm.removeDir(path, false);
        vm._expectCheatcodeRevert();
        vm.fsMetadata(path);

        // reverts because not recursive
        vm._expectCheatcodeRevert();
        vm.createDir(child, false);

        vm.createDir(child, true);
        assertEq(vm.fsMetadata(child).isDir, true);

        // deleted both, recursively
        vm.removeDir(path, true);
        vm._expectCheatcodeRevert();
        vm.fsMetadata(path);
        vm._expectCheatcodeRevert();
        vm.fsMetadata(child);
    }

    function testFsMetadata() public {
        Vm.FsMetadata memory metadata = vm.fsMetadata("fixtures/File");
        assertEq(metadata.isDir, true);
        assertEq(metadata.isSymlink, false);
        assertEq(metadata.readOnly, false);
        // These fields aren't available on all platforms, default to zero
        // assertGt(metadata.length, 0);
        // assertGt(metadata.modified, 0);
        // assertGt(metadata.accessed, 0);
        // assertGt(metadata.created, 0);

        metadata = vm.fsMetadata("fixtures/File/read.txt");
        assertEq(metadata.isDir, false);
        assertEq(metadata.isSymlink, false);
        // This test will fail on windows if we compared to 45, as windows
        // ends files with both line feed and carriage return, unlike
        // unix which only uses the first one.
        assertTrue(metadata.length == 45 || metadata.length == 46);

        metadata = vm.fsMetadata("fixtures/File/symlink");
        assertEq(metadata.isDir, false);
        // TODO: symlinks are canonicalized away in `ensure_path_allowed`
        // assertEq(metadata.isSymlink, true);

        vm._expectCheatcodeRevert();
        vm.fsMetadata("../not-found");

        vm._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        vm.fsMetadata("/etc/hosts");
    }

    function testExists() public {
        string memory validFilePath = "fixtures/File/read.txt";
        assertTrue(vm.exists(validFilePath));
        assertTrue(vm.exists(validFilePath));

        string memory validDirPath = "fixtures/File";
        assertTrue(vm.exists(validDirPath));
        assertTrue(vm.exists(validDirPath));

        string memory invalidPath = "fixtures/File/invalidfile.txt";
        assertTrue(vm.exists(invalidPath) == false);
        assertTrue(vm.exists(invalidPath) == false);
    }

    function testIsFile() public {
        string memory validFilePath = "fixtures/File/read.txt";
        assertTrue(vm.isFile(validFilePath));
        assertTrue(vm.isFile(validFilePath));

        string memory invalidFilePath = "fixtures/File/invalidfile.txt";
        assertTrue(vm.isFile(invalidFilePath) == false);
        assertTrue(vm.isFile(invalidFilePath) == false);

        string memory dirPath = "fixtures/File";
        assertTrue(vm.isFile(dirPath) == false);
        assertTrue(vm.isFile(dirPath) == false);
    }

    function testIsDir() public {
        string memory validDirPath = "fixtures/File";
        assertTrue(vm.isDir(validDirPath));
        assertTrue(vm.isDir(validDirPath));

        string memory invalidDirPath = "fixtures/InvalidDir";
        assertTrue(vm.isDir(invalidDirPath) == false);
        assertTrue(vm.isDir(invalidDirPath) == false);

        string memory filePath = "fixtures/File/read.txt";
        assertTrue(vm.isDir(filePath) == false);
        assertTrue(vm.isDir(filePath) == false);
    }
}
