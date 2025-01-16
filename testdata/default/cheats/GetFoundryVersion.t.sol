// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetFoundryVersionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetFoundryVersion() public view {
        string memory fullVersionString = vm.getFoundryVersion();

        // Ensure the version string contains a "+" separator
        string[] memory versionComponents = vm.split(fullVersionString, "+");
        require(versionComponents.length == 2, "Invalid version format");

        // Validate semantic version (e.g., "0.3.0-stable")
        string memory semanticVersion = versionComponents[0];
        require(bytes(semanticVersion).length > 0, "Semantic version is empty");

        // Validate commit hash (e.g., "3a9576857a")
        string memory commitHash = versionComponents[1];
        require(bytes(commitHash).length > 0, "Commit hash is empty");
    }
}
