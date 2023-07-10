// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract FsTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    bytes constant FOUNDRY_TOML_ACCESS_ERR = "Access to foundry.toml is not allowed.";
    bytes constant FOUNDRY_READ_ERR = "The path \"/etc/hosts\" is not allowed to be accessed for read operations.";
    bytes constant FOUNDRY_READ_DIR_ERR = "The path \"/etc\" is not allowed to be accessed for read operations.";
    bytes constant FOUNDRY_WRITE_ERR = "The path \"/etc/hosts\" is not allowed to be accessed for write operations.";

    function assertEntry(Vm.DirEntry memory entry, uint64 depth, bool dir) private {
        assertEq(entry.errorMessage, "");
        assertEq(entry.depth, depth);
        assertEq(entry.isDir, dir);
        assertEq(entry.isSymlink, false);
    }

    function testReadFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(vm.readFile(path), "hello readable world\nthis is the second line!");

        vm.expectRevert(FOUNDRY_READ_ERR);
        vm.readFile("/etc/hosts");

        vm.expectRevert(FOUNDRY_READ_ERR);
        vm.readFileBinary("/etc/hosts");
    }

    function testReadLine() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(vm.readLine(path), "hello readable world");
        assertEq(vm.readLine(path), "this is the second line!");
        assertEq(vm.readLine(path), "");

        vm.expectRevert(FOUNDRY_READ_ERR);
        vm.readLine("/etc/hosts");
    }

    function testWriteFile() public {
        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm.writeFile(path, data);

        assertEq(vm.readFile(path), data);

        vm.removeFile(path);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.writeFile("/etc/hosts", "malicious stuff");
        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.writeFileBinary("/etc/hosts", "malicious stuff");
    }

    function testWriteLine() public {
        string memory path = "../testdata/fixtures/File/write_line.txt";

        string memory line1 = "first line";
        vm.writeLine(path, line1);

        string memory line2 = "second line";
        vm.writeLine(path, line2);

        assertEq(vm.readFile(path), string.concat(line1, "\n", line2, "\n"));

        vm.removeFile(path);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.writeLine("/etc/hosts", "malicious stuff");
    }

    function testCloseFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(vm.readLine(path), "hello readable world");
        vm.closeFile(path);
        assertEq(vm.readLine(path), "hello readable world");
    }

    function testRemoveFile() public {
        string memory path = "../testdata/fixtures/File/remove_file.txt";
        string memory data = "hello writable world";

        vm.writeFile(path, data);
        assertEq(vm.readLine(path), data);

        vm.removeFile(path);
        vm.writeLine(path, data);
        assertEq(vm.readLine(path), data);

        vm.removeFile(path);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        vm.removeFile("/etc/hosts");
    }

    function testWriteLineFoundrytoml() public {
        string memory root = vm.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");
        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeLine(foundryToml, "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeLine("foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeLine("./foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeLine("./Foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeLine("./../foundry.toml", "\nffi = true\n");
    }

    function testWriteFoundrytoml() public {
        string memory root = vm.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");
        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeFile(foundryToml, "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeFile("foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeFile("./foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeFile("./Foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        vm.writeFile("./../foundry.toml", "\nffi = true\n");
    }

    function testReadDir() public {
        string memory path = "../testdata/fixtures/Dir";

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
            assertEq(contents, unicode"Wow! 😀\n");
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
            assertEntry(entries[4], 3, true);
        }

        vm.expectRevert(FOUNDRY_READ_DIR_ERR);
        vm.readDir("/etc");
    }

    function testCreateRemoveDir() public {
        string memory path = "../testdata/fixtures/Dir/remove_dir";
        string memory child = string.concat(path, "/child");

        vm.createDir(path, false);
        assertEq(vm.fsMetadata(path).isDir, true);

        vm.removeDir(path, false);
        vm.expectRevert();
        vm.fsMetadata(path);

        // reverts because not recursive
        vm.expectRevert();
        vm.createDir(child, false);

        vm.createDir(child, true);
        assertEq(vm.fsMetadata(child).isDir, true);

        // deleted both, recursively
        vm.removeDir(path, true);
        vm.expectRevert();
        vm.fsMetadata(path);
        vm.expectRevert();
        vm.fsMetadata(child);
    }

    function testFsMetadata() public {
        string memory path = "../testdata/fixtures/File";
        Vm.FsMetadata memory metadata = vm.fsMetadata(path);
        assertEq(metadata.isDir, true);
        assertEq(metadata.isSymlink, false);
        assertEq(metadata.readOnly, false);
        assertGt(metadata.length, 0);
        // These fields aren't available on all platforms, default to zero
        // assertGt(metadata.modified, 0);
        // assertGt(metadata.accessed, 0);
        // assertGt(metadata.created, 0);

        path = "../testdata/fixtures/File/read.txt";
        metadata = vm.fsMetadata(path);
        assertEq(metadata.isDir, false);

        path = "../testdata/fixtures/File/symlink";
        metadata = vm.fsMetadata(path);
        assertEq(metadata.isSymlink, true);

        vm.expectRevert();
        vm.fsMetadata("../not-found");

        vm.expectRevert(FOUNDRY_READ_ERR);
        vm.fsMetadata("/etc/hosts");
    }

    // not testing file cheatcodes per se
    function testCheatCodeErrorPrefix() public {
        try vm.readFile("/etc/hosts") {
            emit log("Error: reading /etc/hosts should revert");
            fail();
        } catch (bytes memory err) {
            assertEq(err, abi.encodeWithSignature("CheatCodeError", FOUNDRY_READ_ERR));
        }
    }
}
