// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3674
/// forge-config: default.sender = "0xF0959944122fb1ed4CfaBA645eA06EED30427BAA"
contract Issue3674Test is Test {
    function testNonceCreateSelect() public {
        vm.createSelectFork("sepolia");

        vm.createSelectFork("avaxTestnet");
        assertTrue(vm.getNonce(msg.sender) > 0x17);
    }
}
