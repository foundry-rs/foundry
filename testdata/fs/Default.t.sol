// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

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

contract DefaultAccessTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    FsProxy public fsProxy;

    bytes constant FOUNDRY_WRITE_ERR =
        "The path \"../testdata/fixtures/File/write_file.txt\" is not allowed to be accessed for write operations.";

    function testReadFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";
        vm.readFile(path);

        vm.readFileBinary(path);
    }

    function testReadLine() public {
        string memory path = "../testdata/fixtures/File/read.txt";
        vm.readLine(path);
    }

    function testWriteFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFile(path, data);

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFileBinary(path, bytes(data));
    }

    function testWriteLine() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeLine(path, data);
    }

    function testRemoveFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";

        vm.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.removeFile(path);
    }
}
