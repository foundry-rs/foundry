// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

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

contract DefaultAccessTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    FsProxy public fsProxy;

    bytes constant FOUNDRY_WRITE_ERR =
        "The path \"../testdata/fixtures/File/write_file.txt\" is not allowed to be accessed for write operations.";

    function testReadFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";
        cheats.readFile(path);

        cheats.readFileBinary(path);
    }

    function testReadLine() public {
        string memory path = "../testdata/fixtures/File/read.txt";
        cheats.readLine(path);
    }

    function testWriteFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFile(path, data);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeFileBinary(path, bytes(data));
    }

    function testWriteLine() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.writeLine(path, data);
    }

    function testRemoveFile() public {
        fsProxy = new FsProxy();

        string memory path = "../testdata/fixtures/File/write_file.txt";

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        fsProxy.removeFile(path);
    }
}
