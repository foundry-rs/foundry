// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetFoundryVersionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetFoundryVersion() public view {
        // (e.g. 0.3.0-nightly+3cb96bde9b.1737036656.debug)
        string memory fullVersionString = vm.getFoundryVersion();

        // Step 1: Split the version at "+"
        string[] memory plusSplit = vm.split(fullVersionString, "+");
        require(plusSplit.length == 2, "Invalid version format: Missing '+' separator");

        // Step 2: Extract parts
        string memory semanticVersion = plusSplit[0]; // "0.3.0-dev"
        string memory metadata = plusSplit[1]; // "34389e7850.1737037814.debug"

        // Step 3: Further split metadata by "."
        string[] memory metadataComponents = vm.split(metadata, ".");
        require(metadataComponents.length == 3, "Invalid version format: Metadata should have 3 components");

        // Step 4: Extract values
        string memory commitHash = metadataComponents[0]; // "34389e7850"
        string memory timestamp = metadataComponents[1]; // "1737037814"
        string memory buildType = metadataComponents[2]; // "debug"

        // Validate semantic version (e.g., "0.3.0-stable" or "0.3.0-nightly")
        require(bytes(semanticVersion).length > 0, "Semantic version is empty");

        // Validate commit hash (should be exactly 10 characters)
        require(bytes(commitHash).length == 10, "Invalid commit hash length");

        // Validate UNIX timestamp (numeric)
        uint256 buildUnixTimestamp = vm.parseUint(timestamp);
        uint256 minimumAcceptableTimestamp = 1700000000; // Adjust as needed
        require(buildUnixTimestamp >= minimumAcceptableTimestamp, "Build timestamp is too old");

        // Validate build profile (e.g., "debug" or "release")
        require(bytes(buildType).length > 0, "Build type is empty");
    }
}
