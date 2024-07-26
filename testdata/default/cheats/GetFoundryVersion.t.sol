// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract GetFoundryVersionTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testChainId() public {
        string memory version = vm.getFoundryVersion();
        uint256 buildId = vm.parseUint(vm.split(version, "+")[1]);
        require(buildId >= 202406111234, "too old");
    }
}
