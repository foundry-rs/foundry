// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetFoundryVersionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testGetFoundryVersion() public view {
        string memory version = vm.getFoundryVersion();

        string memory cargoVersion = vm.split(version, "+")[0];
        require(bytes(cargoVersion).length > 0, "Cargo version is empty");

        string memory gitSHA = vm.split(version, "+")[1];
        require(bytes(gitSHA).length > 0, "Git SHA is empty");

        uint256 buildTimestamp = vm.parseUint(vm.split(version, "+")[2]);
        require(buildTimestamp >= 202406111234, "too old");
    }
}
