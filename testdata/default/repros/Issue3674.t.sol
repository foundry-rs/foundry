// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3674
contract Issue3674Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testNonceCreateSelect() public {
        vm.createSelectFork("sepolia");

        vm.createSelectFork("avaxTestnet");
        assert(vm.getNonce(msg.sender) > 0x17);
    }
}
