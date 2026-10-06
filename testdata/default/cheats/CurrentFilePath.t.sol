// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract CurrentFilePathTest is Test {
    function testCurrentFilePath() public {
        string memory filePath = vm.currentFilePath();
        // The path should be relative to the project root and point to this test file.
        assertEq(normalizePath(filePath), "default/cheats/CurrentFilePath.t.sol");
    }

    function testCurrentFilePathIsNotEmpty() public {
        string memory filePath = vm.currentFilePath();
        assertTrue(bytes(filePath).length > 0, "currentFilePath() should not return an empty string");
    }

    function normalizePath(string memory path) internal pure returns (string memory) {
        return vm.replace(path, "\\", "/");
    }
}
