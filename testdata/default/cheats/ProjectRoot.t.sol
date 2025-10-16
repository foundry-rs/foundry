// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ProjectRootTest is Test {
    bytes public manifestDirBytes;

    function testProjectRoot() public {
        // .../crates/forge
        string memory manifestDir = vm.envString("CARGO_MANIFEST_DIR");
        string memory manifestDirNormalized = vm.replace(manifestDir, "\\", "/");
        manifestDirBytes = bytes(manifestDirNormalized);
        
        for (uint256 i = 0; i < 7; i++) {
            manifestDirBytes.pop();
        }
        // replace "forge" suffix with "testdata" suffix to get expected project root from manifest dir
        bytes memory expectedRootSuffix = bytes("testd");
        for (uint256 i = 1; i < 6; i++) {
            manifestDirBytes[manifestDirBytes.length - i] = expectedRootSuffix[expectedRootSuffix.length - i];
        }
        bytes memory expectedRootDir = abi.encodePacked(manifestDirBytes, "ata");

        assertEq(vm.projectRoot(), string(expectedRootDir));
    }
}
