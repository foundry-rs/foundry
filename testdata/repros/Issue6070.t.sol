// SPDX-License-Identifier: MIT
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "../cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6070
contract Issue6066Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testNonPrefixed() public {
        vm.setEnv("__FOUNDRY_ISSUE_6066", "abcd");
        vm.expectRevert("failed parsing \"abcd\" as type `uint256`: missing hex prefix (\"0x\") for hex string");
        uint256 x = vm.envUint("__FOUNDRY_ISSUE_6066");
    }
}
