// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract ProjectRootTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    bytes public manifestDirBytes;

    function testProjectRoot() public {
        manifestDirBytes = bytes(vm.envString("CARGO_MANIFEST_DIR"));
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
