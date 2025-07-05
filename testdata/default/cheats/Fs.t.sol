// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract FsTest is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);
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

        assertEq(VM.readFile(path), "hello readable world\nthis is the second line!");

        VM._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        VM.readFile("/etc/hosts");

        VM._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        VM.readFileBinary("/etc/hosts");
    }

    function testReadLine() public {
        string memory path = "fixtures/File/read.txt";

        assertEq(VM.readLine(path), "hello readable world");
        assertEq(VM.readLine(path), "this is the second line!");
        assertEq(VM.readLine(path), "");

        VM._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        VM.readLine("/etc/hosts");
    }

    function testWriteFile() public {
        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        VM.writeFile(path, data);

        assertEq(VM.readFile(path), data);

        VM.removeFile(path);

        VM._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        VM.writeFile("/etc/hosts", "malicious stuff");
        VM._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        VM.writeFileBinary("/etc/hosts", "malicious stuff");
    }

    function testCopyFile() public {
        string memory from = "fixtures/File/read.txt";
        string memory to = "fixtures/File/copy.txt";
        uint64 copied = VM.copyFile(from, to);
        assertEq(VM.fsMetadata(to).length, uint256(copied));
        assertEq(VM.readFile(from), VM.readFile(to));
        VM.removeFile(to);
    }

    function testWriteLine() public {
        string memory path = "fixtures/File/write_line.txt";

        string memory line1 = "first line";
        VM.writeLine(path, line1);

        string memory line2 = "second line";
        VM.writeLine(path, line2);

        assertEq(VM.readFile(path), string.concat(line1, "\n", line2, "\n"));

        VM.removeFile(path);

        VM._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        VM.writeLine("/etc/hosts", "malicious stuff");
    }

    function testCloseFile() public {
        string memory path = "fixtures/File/read.txt";

        assertEq(VM.readLine(path), "hello readable world");
        VM.closeFile(path);
        assertEq(VM.readLine(path), "hello readable world");
    }

    function testRemoveFile() public {
        string memory path = "fixtures/File/remove_file.txt";
        string memory data = "hello writable world";

        VM.writeFile(path, data);
        assertEq(VM.readLine(path), data);

        VM.removeFile(path);
        VM.writeLine(path, data);
        assertEq(VM.readLine(path), data);

        VM.removeFile(path);

        VM._expectCheatcodeRevert(FOUNDRY_WRITE_ERR);
        VM.removeFile("/etc/hosts");
    }

    function testWriteLineFoundrytoml() public {
        string memory root = VM.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        VM._expectCheatcodeRevert();
        VM.writeLine(foundryToml, "\nffi = true\n");

        VM._expectCheatcodeRevert();
        VM.writeLine("foundry.toml", "\nffi = true\n");

        VM._expectCheatcodeRevert();
        VM.writeLine("./foundry.toml", "\nffi = true\n");

        VM._expectCheatcodeRevert();
        VM.writeLine("./Foundry.toml", "\nffi = true\n");
    }

    function testWriteFoundrytoml() public {
        string memory root = VM.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        VM._expectCheatcodeRevert();
        VM.writeFile(foundryToml, "\nffi = true\n");

        VM._expectCheatcodeRevert();
        VM.writeFile("foundry.toml", "\nffi = true\n");

        VM._expectCheatcodeRevert();
        VM.writeFile("./foundry.toml", "\nffi = true\n");

        VM._expectCheatcodeRevert();
        VM.writeFile("./Foundry.toml", "\nffi = true\n");
    }

    function testReadDir() public {
        string memory path = "fixtures/Dir";

        {
            Vm.DirEntry[] memory entries = VM.readDir(path);
            assertEq(entries.length, 2);
            assertEntry(entries[0], 1, false);
            assertEntry(entries[1], 1, true);

            Vm.DirEntry[] memory entries2 = VM.readDir(path, 1);
            assertEq(entries2.length, 2);
            assertEq(entries[0].path, entries2[0].path);
            assertEq(entries[1].path, entries2[1].path);

            string memory contents = VM.readFile(entries[0].path);
            assertEq(contents, unicode"Wow! 😀");
        }

        {
            Vm.DirEntry[] memory entries = VM.readDir(path, 2);
            assertEq(entries.length, 4);
            assertEntry(entries[2], 2, false);
            assertEntry(entries[3], 2, true);
        }

        {
            Vm.DirEntry[] memory entries = VM.readDir(path, 3);
            assertEq(entries.length, 5);
            assertEntry(entries[4], 3, false);
        }

        VM._expectCheatcodeRevert(FOUNDRY_READ_DIR_ERR);
        VM.readDir("/etc");
    }

    function testCreateRemoveDir() public {
        string memory path = "fixtures/Dir/remove_dir";
        string memory child = string.concat(path, "/child");

        VM.createDir(path, false);
        assertEq(VM.fsMetadata(path).isDir, true);

        VM.removeDir(path, false);
        VM._expectCheatcodeRevert();
        VM.fsMetadata(path);

        // reverts because not recursive
        VM._expectCheatcodeRevert();
        VM.createDir(child, false);

        VM.createDir(child, true);
        assertEq(VM.fsMetadata(child).isDir, true);

        // deleted both, recursively
        VM.removeDir(path, true);
        VM._expectCheatcodeRevert();
        VM.fsMetadata(path);
        VM._expectCheatcodeRevert();
        VM.fsMetadata(child);
    }

    function testFsMetadata() public {
        Vm.FsMetadata memory metadata = VM.fsMetadata("fixtures/File");
        assertEq(metadata.isDir, true);
        assertEq(metadata.isSymlink, false);
        assertEq(metadata.readOnly, false);
        // These fields aren't available on all platforms, default to zero
        // assertGt(metadata.length, 0);
        // assertGt(metadata.modified, 0);
        // assertGt(metadata.accessed, 0);
        // assertGt(metadata.created, 0);

        metadata = VM.fsMetadata("fixtures/File/read.txt");
        assertEq(metadata.isDir, false);
        assertEq(metadata.isSymlink, false);
        // This test will fail on windows if we compared to 45, as windows
        // ends files with both line feed and carriage return, unlike
        // unix which only uses the first one.
        assertTrue(metadata.length == 45 || metadata.length == 46);

        metadata = VM.fsMetadata("fixtures/File/symlink");
        assertEq(metadata.isDir, false);
        // TODO: symlinks are canonicalized away in `ensure_path_allowed`
        // assertEq(metadata.isSymlink, true);

        VM._expectCheatcodeRevert();
        VM.fsMetadata("../not-found");

        VM._expectCheatcodeRevert(FOUNDRY_READ_ERR);
        VM.fsMetadata("/etc/hosts");
    }

    function testExists() public {
        string memory validFilePath = "fixtures/File/read.txt";
        assertTrue(VM.exists(validFilePath));
        assertTrue(VM.exists(validFilePath));

        string memory validDirPath = "fixtures/File";
        assertTrue(VM.exists(validDirPath));
        assertTrue(VM.exists(validDirPath));

        string memory invalidPath = "fixtures/File/invalidfile.txt";
        assertTrue(VM.exists(invalidPath) == false);
        assertTrue(VM.exists(invalidPath) == false);
    }

    function testIsFile() public {
        string memory validFilePath = "fixtures/File/read.txt";
        assertTrue(VM.isFile(validFilePath));
        assertTrue(VM.isFile(validFilePath));

        string memory invalidFilePath = "fixtures/File/invalidfile.txt";
        assertTrue(VM.isFile(invalidFilePath) == false);
        assertTrue(VM.isFile(invalidFilePath) == false);

        string memory dirPath = "fixtures/File";
        assertTrue(VM.isFile(dirPath) == false);
        assertTrue(VM.isFile(dirPath) == false);
    }

    function testIsDir() public {
        string memory validDirPath = "fixtures/File";
        assertTrue(VM.isDir(validDirPath));
        assertTrue(VM.isDir(validDirPath));

        string memory invalidDirPath = "fixtures/InvalidDir";
        assertTrue(VM.isDir(invalidDirPath) == false);
        assertTrue(VM.isDir(invalidDirPath) == false);

        string memory filePath = "fixtures/File/read.txt";
        assertTrue(VM.isDir(filePath) == false);
        assertTrue(VM.isDir(filePath) == false);
    }
}
