// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract ProjectRootTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testProjectRoot() public {
        bytes memory manifestDirBytes = bytes(cheats.envString("CARGO_MANIFEST_DIR"));

        // replace "forge" suffix with "testdata" suffix to get expected project root from manifest dir
        bytes memory expectedRootSuffix = bytes("testd");
        for (uint256 i = 1; i < 6; i++) {
            manifestDirBytes[manifestDirBytes.length - i] = expectedRootSuffix[expectedRootSuffix.length - i];
        }
        bytes memory expectedRootDir = abi.encodePacked(manifestDirBytes, "ata");

        assertEq(cheats.projectRoot(), string(expectedRootDir));
    }
}
