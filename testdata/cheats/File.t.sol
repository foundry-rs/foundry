// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract FileTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testReadFile() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readFile(path), "hello readable world\nthis is the second line!");
    }

    function testReadLine() public {
        string memory path = "../testdata/fixtures/File/read.txt";

        assertEq(cheats.readLine(path), "hello readable world");
        assertEq(cheats.readLine(path), "this is the second line!");
        assertEq(cheats.readLine(path), "");
    }

    function testWriteFile() public {
        string memory path = "../testdata/fixtures/File/write_file.txt";
        string memory data = "hello writable world";
        cheats.writeFile(path, data);

        assertEq(cheats.readFile(path), data);

        cheats.removeFile(path);
    }

    function testWriteLine() public {
        string memory path = "../testdata/fixtures/File/write_line.txt";

        string memory line1 = "first line";
        cheats.writeLine(path, line1);

        string memory line2 = "second line";
        cheats.writeLine(path, line2);

        assertEq(cheats.readFile(path), string.concat(line1, "\n", line2, "\n"));

        cheats.removeFile(path);
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
    }
}
