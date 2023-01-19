// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract FileTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    bytes constant FOUNDRY_TOML_ACCESS_ERR = "Access to foundry.toml is not allowed.";
    bytes constant FOUNDRY_READ_ERR = "The path \"/etc/hosts\" is not allowed to be accessed for read operations.";
    bytes constant FOUNDRY_WRITE_ERR = "The path \"/etc/hosts\" is not allowed to be accessed for write operations.";

    function testReadFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readFile(path), "hello readable world\nthis is the second line!");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        cheats.readFile("/etc/hosts");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        cheats.readFileBinary("/etc/hosts");
    }

    function testReadLine() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readLine(path), "hello readable world");
        assertEq(cheats.readLine(path), "this is the second line!");
        assertEq(cheats.readLine(path), "");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        cheats.readLine("/etc/hosts");
    }

    function testWriteFile() public {
        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        cheats.writeFile(path, data);

        assertEq(cheats.readFile(path), data);

        cheats.removeFile(path);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        cheats.writeFile("/etc/hosts", "malicious stuff");
        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        cheats.writeFileBinary("/etc/hosts", "malicious stuff");
    }

    function testWriteLine() public {
        string memory path = "../testdata/fixtures/File/write_line.txt";

        string memory line1 = "first line";
        cheats.writeLine(path, line1);

        string memory line2 = "second line";
        cheats.writeLine(path, line2);

        assertEq(cheats.readFile(path), string.concat(line1, "\n", line2, "\n"));

        cheats.removeFile(path);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        cheats.writeLine("/etc/hosts", "malicious stuff");
    }

    function testCloseFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readLine(path), "hello readable world");
        cheats.closeFile(path);
        assertEq(cheats.readLine(path), "hello readable world");
    }

    function testRemoveFile() public {
        string memory path = "../testdata/fixtures/File/remove_file.txt";
        string memory data = "hello writable world";

        cheats.writeFile(path, data);
        assertEq(cheats.readLine(path), data);

        cheats.removeFile(path);
        cheats.writeLine(path, data);
        assertEq(cheats.readLine(path), data);

        cheats.removeFile(path);

        cheats.expectRevert(FOUNDRY_WRITE_ERR);
        cheats.removeFile("/etc/hosts");
    }

    function testWriteLineFoundrytoml() public {
        string memory root = cheats.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");
        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeLine(foundryToml, "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeLine("foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeLine("./foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeLine("./Foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeLine("./../foundry.toml", "\nffi = true\n");
    }

    function testWriteFoundrytoml() public {
        string memory root = cheats.projectRoot();
        string memory foundryToml = string.concat(root, "/", "foundry.toml");
        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeFile(foundryToml, "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeFile("foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeFile("./foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeFile("./Foundry.toml", "\nffi = true\n");

        cheats.expectRevert(FOUNDRY_TOML_ACCESS_ERR);
        cheats.writeFile("./../foundry.toml", "\nffi = true\n");
    }

    function testFsMetadata() public {
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
        assertEq(metadata.isSymlink, true);

        cheats.expectRevert();
        cheats.fsMetadata("../not-found");

        cheats.expectRevert(FOUNDRY_READ_ERR);
        cheats.fsMetadata("/etc/hosts");
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
