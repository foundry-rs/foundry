// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3674
contract Issue3674Test is Test {
    function testNonceCreateSelect() public {
        vm.createSelectFork("sepolia");

        vm.createSelectFork("avaxTestnet");
        assert(vm.getNonce(msg.sender) > 0x17);
    }
}
