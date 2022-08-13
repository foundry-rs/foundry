// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract ProjectRootTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testProjectRoot() public {
        bytes memory manifestDirBytes = bytes(cheats.envString("CARGO_MANIFEST_DIR"));

        // replace "forge" with "testdata" suffix to get expected project root from manifest dir
        bytes memory expectedRootSuffix = bytes("testd");
        for(uint i = 1; i < 6; i++) {
            manifestDirBytes[manifestDirBytes.length - i] = expectedRootSuffix[expectedRootSuffix.length - i];
        }
        bytes memory expectedRootDir = abi.encodePacked(manifestDirBytes, "ata");

        assertEq(cheats.projectRoot(), string(expectedRootDir));
    }

    function substring(string memory str, uint startIndex, uint endIndex) public pure returns (string memory) {
      bytes memory strBytes = bytes(str);
      bytes memory result = new bytes(endIndex-startIndex);
      for(uint i = startIndex; i < endIndex; i++) {
          result[i-startIndex] = strBytes[i];
      }
      return string(result);
}
}
