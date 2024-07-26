// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetFoundryVersionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetFoundryVersion() public view {
        string memory fullVersionString = vm.getFoundryVersion();

        string[] memory versionComponents = vm.split(fullVersionString, "+");
        require(versionComponents.length == 3, "Invalid version format");

        string memory semanticVersion = versionComponents[0];
        require(bytes(semanticVersion).length > 0, "Semantic version is empty");

        string memory commitHash = versionComponents[1];
        require(bytes(commitHash).length > 0, "Commit hash is empty");

        uint256 buildUnixTimestamp = vm.parseUint(versionComponents[2]);
        uint256 minimumAcceptableTimestamp = 202406111234;
        require(buildUnixTimestamp >= minimumAcceptableTimestamp, "Build timestamp is too old");
    }
}
