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
        require(versionComponents.length >= 3, "Invalid version format");

        // Validate semantic version (e.g., "0.3.0-stable")
        string memory semanticVersion = versionComponents[0];
        require(bytes(semanticVersion).length > 0, "Semantic version is empty");

        // Validate commit hash (e.g., "3a9576857a")
        string memory commitHash = versionComponents[1];
        require(bytes(commitHash).length > 0, "Commit hash is empty");

        // Validate timestamp (e.g., "20250116")
        string memory timestamp = versionComponents[2];
        require(bytes(timestamp).length == 8, "Invalid timestamp format");

        // Validate build type (optional, e.g., "debug" or "release")
        if (versionComponents.length > 3) {
            string memory buildType = versionComponents[3];
            require(bytes(buildType).length > 0, "Build type is empty");
        }
    }
}
