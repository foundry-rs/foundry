// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract FsProxy is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function readFile(string calldata path) external returns (string memory) {
        return cheats.readFile(path);
    }

    function readDir(string calldata path) external returns (Cheats.DirEntry[] memory) {
        return cheats.readDir(path);
    }

    function readFileBinary(string calldata path) external returns (bytes memory) {
        return cheats.readFileBinary(path);
    }

    function readLine(string calldata path) external returns (string memory) {
        return cheats.readLine(path);
    }

    function writeLine(string calldata path, string calldata data) external {
        return cheats.writeLine(path, data);
    }

    function writeFile(string calldata path, string calldata data) external {
        return cheats.writeLine(path, data);
    }

    function writeFileBinary(string calldata path, bytes calldata data) external {
        return cheats.writeFileBinary(path, data);
    }

    function removeFile(string calldata path) external {
        return cheats.removeFile(path);
    }

    function fsMetadata(string calldata path) external returns (Cheats.FsMetadata memory) {
        return cheats.fsMetadata(path);
    }

    function createDir(string calldata path) external {
        return cheats.createDir(path, false);
    }

    function createDir(string calldata path, bool recursive) external {
        return cheats.createDir(path, recursive);
    }
}

contract FsTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    FsProxy public fsProxy;
    bytes constant FOUNDRY_TOML_ACCESS_ERR = "Access to foundry.toml is not allowed.";
    bytes constant FOUNDRY_READ_ERR = "The path \"/etc/hosts\" is not allowed to be accessed for read operations.";
    bytes constant FOUNDRY_READ_DIR_ERR = "The path \"/etc\" is not allowed to be accessed for read operations.";
    bytes constant FOUNDRY_WRITE_ERR = "The path \"/etc/hosts\" is not allowed to be accessed for write operations.";

    function assertEntry(Cheats.DirEntry memory entry, uint64 depth, bool dir) private {
        assertEq(entry.errorMessage, "");
        assertEq(entry.depth, depth);
        assertEq(entry.isDir, dir);
        assertEq(entry.isSymlink, false);
    }

    function testReadFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readFile(path), "hello readable world\nthis is the second line!");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.readFile("/etc/hosts");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.readFileBinary("/etc/hosts");
    }

    function testReadLine() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readLine(path), "hello readable world");
        assertEq(cheats.readLine(path), "this is the second line!");
        assertEq(cheats.readLine(path), "");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.readLine("/etc/hosts");
    }

    function testWriteFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        cheats.writeFile(path, data);

        assertEq(cheats.readFile(path), data);

        cheats.removeFile(path);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFile("/etc/hosts", "malicious stuff");
        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFileBinary("/etc/hosts", "malicious stuff");
    }

    function testWriteLine() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_line.txt";

        string memory line1 = "first line";
        cheats.writeLine(path, line1);

        string memory line2 = "second line";
        cheats.writeLine(path, line2);

        assertEq(cheats.readFile(path), string.concat(line1, "\n", line2, "\n"));

        cheats.removeFile(path);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeLine("/etc/hosts", "malicious stuff");
    }

    function testCloseFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readLine(path), "hello readable world");
        cheats.closeFile(path);
        assertEq(cheats.readLine(path), "hello readable world");
    }

    function testRemoveFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/remove_file.txt";
        string memory data = "hello writable world";

        cheats.writeFile(path, data);
        assertEq(cheats.readLine(path), data);

        cheats.removeFile(path);
        cheats.writeLine(path, data);
        assertEq(cheats.readLine(path), data);

        cheats.removeFile(path);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.removeFile("/etc/hosts");
    }

    function testWriteLineFoundrytoml() public {
        fsProxy = new FsProxy();

        string memory root = cheats.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine(foundryToml, "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine("foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine("./foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine("./Foundry.toml", "\nffi = true\n");

        // TODO: This test is not working properly,
        // This writeFile call is not reverting as it should and therefore it's
        // writing to the foundry.toml file
        // cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        // fsProxy.writeLine("./../foundry.toml", "\nffi = true\n");
    }

    function testWriteFoundrytoml() public {
        fsProxy = new FsProxy();

        string memory root = cheats.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile(foundryToml, "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile("foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile("./foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile("./Foundry.toml", "\nffi = true\n");

        // TODO: This test is not working properly,
        // This writeFile call is not reverting as it should and therefore it's
        // writing to the foundry.toml file
        // cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        // fsProxy.writeFile("./../foundry.toml", "\nffi = true\n");
    }

    function testReadDir() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/Dir";

        {
            Cheats.DirEntry[] memory entries = cheats.readDir(path);
            assertEq(entries.length, 2);
            assertEntry(entries[0], 1, false);
            assertEntry(entries[1], 1, true);

            Cheats.DirEntry[] memory entries2 = cheats.readDir(path, 1);
            assertEq(entries2.length, 2);
            assertEq(entries[0].path, entries2[0].path);
            assertEq(entries[1].path, entries2[1].path);

            string memory contents = cheats.readFile(entries[0].path);
            assertEq(contents, unicode"Wow! 😀\n");
        }

        {
            Cheats.DirEntry[] memory entries = cheats.readDir(path, 2);
            assertEq(entries.length, 4);
            assertEntry(entries[2], 2, false);
            assertEntry(entries[3], 2, true);
        }

        {
            Cheats.DirEntry[] memory entries = cheats.readDir(path, 3);
            assertEq(entries.length, 5);
            assertEntry(entries[4], 3, false);
        }

        cheats.expectRevert(FOUNDRY_READ_DIR_ERR);
        fsProxy.readDir("/etc");
    }

    function testCreateRemoveDir() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/Dir/remove_dir";
        string memory child = string.concat(path, "/child");

        cheats.createDir(path, false);
        assertEq(cheats.fsMetadata(path).isDir, true);

        cheats.removeDir(path, false);
        cheats.expectRevert();
        fsProxy.fsMetadata(path);

        // reverts because not recursive
        cheats.expectRevert();
        fsProxy.createDir(child, false);

        cheats.createDir(child, true);
        assertEq(cheats.fsMetadata(child).isDir, true);

        // deleted both, recursively
        cheats.removeDir(path, true);
        cheats.expectRevert();
        fsProxy.fsMetadata(path);
        cheats.expectRevert();
        fsProxy.fsMetadata(child);
    }

    function testFsMetadata() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File";
        Cheats.FsMetadata memory metadata = cheats.fsMetadata(path);
        assertEq(metadata.isDir, true);
        assertEq(metadata.isSymlink, false);
        assertEq(metadata.readOnly, false);
        assertGt(metadata.length, 0);
        // These fields aren't available on all platforms, default to zero
        // assertGt(metadata.modified, 0);
        // assertGt(metadata.accessed, 0);
        // assertGt(metadata.created, 0);

        path = "../testdata/fixtures/File/read.txt";
        metadata = cheats.fsMetadata(path);
        assertEq(metadata.isDir, false);

        path = "../testdata/fixtures/File/symlink";
        metadata = cheats.fsMetadata(path);
        assertEq(metadata.isSymlink, false);

        cheats.expectRevert();
        fsProxy.fsMetadata("../not-found");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.fsMetadata("/etc/hosts");
    }

    // not testing file cheatcodes per se
    function testCheatCodeErrorPrefix() public {
        try cheats.readFile("/etc/hosts") {
            emit log("Error: reading /etc/hosts should revert");
            fail();
        } catch (bytes memory err) {
            assertEq(err, abi.encodeWithSignature("CheatCodeError", FOUNDRY_READ_ERR));
        }
    }
}
