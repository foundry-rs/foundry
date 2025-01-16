// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetFoundryVersionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetFoundryVersion() public view {
        // (e.g. 0.3.0-nightly+3cb96bde9b.1737036656.debug)
        string memory fullVersionString = vm.getFoundryVersion();

        // Ensure the version string contains at least four components after splitting by "+"
        string[] memory versionComponents = vm.split(fullVersionString, "+");
        require(versionComponents.length >= 4, "Invalid version format");

        // Validate semantic version (e.g., "0.3.0-stable" or "0.3.0-nightly")
        string memory semanticVersion = versionComponents[0];
        require(bytes(semanticVersion).length > 0, "Semantic version is empty");

        // Validate commit hash (e.g., "3cb96bde9b")
        string memory commitHash = versionComponents[1];
        require(bytes(commitHash).length == 10, "Invalid commit hash length");

        // Validate UNIX timestamp (e.g., "1737036656")
        uint256 buildUnixTimestamp = vm.parseUint(versionComponents[2]);
        uint256 minimumAcceptableTimestamp = 1700000000; // Adjust as needed
        require(buildUnixTimestamp >= minimumAcceptableTimestamp, "Build timestamp is too old");

        // Validate build profile (optional, e.g., "debug" or "release")
        if (versionComponents.length > 3) {
            string memory buildType = versionComponents[3];
            require(bytes(buildType).length > 0, "Build type is empty");
        }
    }
}
