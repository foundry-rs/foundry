// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract FsProxy is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function readFile(string calldata path) external returns (string memory) {
        return vm.readFile(path);
    }

    function readDir(string calldata path) external returns (Vm.DirEntry[] memory) {
        return vm.readDir(path);
    }

    function readFileBinary(string calldata path) external returns (bytes memory) {
        return vm.readFileBinary(path);
    }

    function readLine(string calldata path) external returns (string memory) {
        return vm.readLine(path);
    }

    function writeLine(string calldata path, string calldata data) external {
        return vm.writeLine(path, data);
    }

    function writeFile(string calldata path, string calldata data) external {
        return vm.writeLine(path, data);
    }

    function writeFileBinary(string calldata path, bytes calldata data) external {
        return vm.writeFileBinary(path, data);
    }

    function removeFile(string calldata path) external {
        return vm.removeFile(path);
    }

    function fsMetadata(string calldata path) external returns (Vm.FsMetadata memory) {
        return vm.fsMetadata(path);
    }

    function createDir(string calldata path) external {
        return vm.createDir(path, false);
    }

    function createDir(string calldata path, bool recursive) external {
        return vm.createDir(path, recursive);
    }
}

contract FsTest is DSTest {
    FsProxy public fsProxy;
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
        fsProxy = new FsProxy();

        string memory path = "fixtures/File/read.txt";

        assertEq(vm.readFile(path), "hello readable world\nthis is the second line!");

        vm.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.readFile("/etc/hosts");

        vm.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.readFileBinary("/etc/hosts");
    }

    function testReadLine() public {
        fsProxy = new FsProxy();

        string memory path = "fixtures/File/read.txt";

        assertEq(vm.readLine(path), "hello readable world");
        assertEq(vm.readLine(path), "this is the second line!");
        assertEq(vm.readLine(path), "");

        vm.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.readLine("/etc/hosts");
    }

    function testWriteFile() public {
        fsProxy = new FsProxy();

        string memory path = "fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        vm.writeFile(path, data);

        assertEq(vm.readFile(path), data);

        vm.removeFile(path);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFile("/etc/hosts", "malicious stuff");
        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFileBinary("/etc/hosts", "malicious stuff");
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
        fsProxy = new FsProxy();

        string memory path = "fixtures/File/write_line.txt";

        string memory line1 = "first line";
        vm.writeLine(path, line1);

        string memory line2 = "second line";
        vm.writeLine(path, line2);

        assertEq(vm.readFile(path), string.concat(line1, "\n", line2, "\n"));

        vm.removeFile(path);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeLine("/etc/hosts", "malicious stuff");
    }

    function testCloseFile() public {
        string memory path = "fixtures/File/read.txt";

        assertEq(vm.readLine(path), "hello readable world");
        vm.closeFile(path);
        assertEq(vm.readLine(path), "hello readable world");
    }

    function testRemoveFile() public {
        fsProxy = new FsProxy();

        string memory path = "fixtures/File/remove_file.txt";
        string memory data = "hello writable world";

        vm.writeFile(path, data);
        assertEq(vm.readLine(path), data);

        vm.removeFile(path);
        vm.writeLine(path, data);
        assertEq(vm.readLine(path), data);

        vm.removeFile(path);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.removeFile("/etc/hosts");
    }

    function testWriteLineFoundrytoml() public {
        fsProxy = new FsProxy();

        string memory root = vm.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine(foundryToml, "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine("foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine("./foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeLine("./Foundry.toml", "\nffi = true\n");
    }

    function testWriteFoundrytoml() public {
        fsProxy = new FsProxy();

        string memory root = vm.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile(foundryToml, "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile("foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile("./foundry.toml", "\nffi = true\n");

        vm.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        fsProxy.writeFile("./Foundry.toml", "\nffi = true\n");
    }

    function testReadDir() public {
        fsProxy = new FsProxy();

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
            assertEntry(entries[4], 3, false);
        }

        vm.expectRevert(FOUNDRY_READ_DIR_ERR);
        fsProxy.readDir("/etc");
    }

    function testCreateRemoveDir() public {
        fsProxy = new FsProxy();

        string memory path = "fixtures/Dir/remove_dir";
        string memory child = string.concat(path, "/child");

        vm.createDir(path, false);
        assertEq(vm.fsMetadata(path).isDir, true);

        vm.removeDir(path, false);
        vm.expectRevert();
        fsProxy.fsMetadata(path);

        // reverts because not recursive
        vm.expectRevert();
        fsProxy.createDir(child, false);

        vm.createDir(child, true);
        assertEq(vm.fsMetadata(child).isDir, true);

        // deleted both, recursively
        vm.removeDir(path, true);
        vm.expectRevert();
        fsProxy.fsMetadata(path);
        vm.expectRevert();
        fsProxy.fsMetadata(child);
    }

    function testFsMetadata() public {
        fsProxy = new FsProxy();

        string memory path = "fixtures/File";
        Vm.FsMetadata memory metadata = vm.fsMetadata(path);
        assertEq(metadata.isDir, true);
        assertEq(metadata.isSymlink, false);
        assertEq(metadata.readOnly, false);
        assertGt(metadata.length, 0);
        // These fields aren't available on all platforms, default to zero
        // assertGt(metadata.modified, 0);
        // assertGt(metadata.accessed, 0);
        // assertGt(metadata.created, 0);

        path = "fixtures/File/read.txt";
        metadata = vm.fsMetadata(path);
        assertEq(metadata.isDir, false);

        path = "fixtures/File/symlink";
        metadata = vm.fsMetadata(path);
        assertEq(metadata.isSymlink, false);

        vm.expectRevert();
        fsProxy.fsMetadata("../not-found");

        vm.expectRevert(FOUNDRY_READ_ERR);
        fsProxy.fsMetadata("/etc/hosts");
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
