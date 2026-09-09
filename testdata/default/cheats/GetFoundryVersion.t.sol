// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract GetFoundryVersionTest is Test {
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

    function testFoundryVersionCmp() public {
        // Should return -1 if current version is less than argument
        assertEq(vm.foundryVersionCmp("99.0.0"), -1);

        // (e.g. 0.3.0-nightly+3cb96bde9b.1737036656.debug)
        string memory fullVersionString = vm.getFoundryVersion();

        // Step 1: Split the version at "+"
        string[] memory plusSplit = vm.split(fullVersionString, "+");
        require(plusSplit.length == 2, "Invalid version format: Missing '+' separator");

        // Step 2: Extract parts
        string memory semanticVersion = plusSplit[0]; // "0.3.0-dev"
        string[] memory semanticSplit = vm.split(semanticVersion, "-");

        semanticVersion = semanticSplit[0]; // "0.3.0"
        // Should return 0 if current version is equal to argument
        assertEq(vm.foundryVersionCmp(semanticVersion), 0);

        // Should return 1 if current version is greater than argument
        assertEq(vm.foundryVersionCmp("0.0.1"), 1);
    }

    function testFoundryVersionAtLeast() public {
        // Should return false for future versions
        assertEq(vm.foundryVersionAtLeast("99.0.0"), false);

        // (e.g. 0.3.0-nightly+3cb96bde9b.1737036656.debug)
        string memory fullVersionString = vm.getFoundryVersion();

        // Step 1: Split the version at "+"
        string[] memory plusSplit = vm.split(fullVersionString, "+");
        require(plusSplit.length == 2, "Invalid version format: Missing '+' separator");

        // Step 2: Extract parts
        string memory semanticVersion = plusSplit[0]; // "0.3.0-dev"
        string[] memory semanticSplit = vm.split(semanticVersion, "-");

        semanticVersion = semanticSplit[0]; // "0.3.0"
        assertTrue(vm.foundryVersionAtLeast(semanticVersion));

        // Should return true for past versions
        assertTrue(vm.foundryVersionAtLeast("0.2.0"));
    }

    /// Returns the `MAJOR.MINOR.PATCH` prefix of `vm.getFoundryVersion()`,
    /// stripping any pre-release suffix (`-nightly`, `-dev`, …) and the
    /// `+<sha>.<ts>.<profile>` build metadata.
    function _semverPrefix() internal view returns (string memory) {
        string[] memory plusSplit = vm.split(vm.getFoundryVersion(), "+");
        require(plusSplit.length == 2, "Invalid version format: Missing '+' separator");
        string[] memory dashSplit = vm.split(plusSplit[0], "-");
        return dashSplit[0];
    }

    function testGetFoundryVersionMajorMinorPatchIsParseable() public view {
        // The MAJOR.MINOR.PATCH prefix must always be three numeric components,
        // regardless of build kind (tagged release / nightly / dev).
        string[] memory parts = vm.split(_semverPrefix(), ".");
        require(parts.length == 3, "Invalid semver prefix: expected MAJOR.MINOR.PATCH");
        // Each component must parse as a uint (this reverts on garbage).
        vm.parseUint(parts[0]);
        vm.parseUint(parts[1]);
        vm.parseUint(parts[2]);
    }

    function testGetFoundryVersionBuildProfile() public view {
        // The build profile must be present and non-empty (e.g. "debug", "release", "dist", …).
        string[] memory plusSplit = vm.split(vm.getFoundryVersion(), "+");
        string[] memory metadataComponents = vm.split(plusSplit[1], ".");
        require(bytes(metadataComponents[2]).length > 0, "Build profile is empty");
    }

    function testFoundryVersionCmpAndAtLeastAreConsistent() public {
        // `foundryVersionAtLeast(v)` must equal `foundryVersionCmp(v) >= 0` for any input.
        string[3] memory probes = ["0.0.1", _semverPrefix(), "99.0.0"];
        for (uint256 i = 0; i < probes.length; i++) {
            assertEq(vm.foundryVersionAtLeast(probes[i]), vm.foundryVersionCmp(probes[i]) >= 0);
        }
    }

    function testFoundryVersionCmpRejectsPreRelease() public {
        vm._expectCheatcodeRevert();
        vm.foundryVersionCmp("1.0.0-nightly");
    }

    function testFoundryVersionCmpRejectsBuildMetadata() public {
        vm._expectCheatcodeRevert();
        vm.foundryVersionCmp("1.0.0+abc1234567.1700000000.release");
    }

    function testFoundryVersionCmpRejectsInvalidVersion() public {
        vm._expectCheatcodeRevert();
        vm.foundryVersionCmp("not-a-version");
    }
}
