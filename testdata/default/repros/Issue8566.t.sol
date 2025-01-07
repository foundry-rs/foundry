// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/8566
contract Issue8566Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testParseJsonUint() public {
        string memory json =
            "{ \"1284\": { \"addRewardInfo\": { \"amount\": 74258.225772486694040708e18, \"rewardPerSec\": 0.03069536448928848133e20 } } }";

        assertEq(74258225772486694040708, vm.parseJsonUint(json, ".1284.addRewardInfo.amount"));
        assertEq(3069536448928848133, vm.parseJsonUint(json, ".1284.addRewardInfo.rewardPerSec"));
    }
}
