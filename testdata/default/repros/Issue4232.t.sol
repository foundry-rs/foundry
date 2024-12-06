// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/4232
contract Issue4232Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testFork() public {
        // Smoke test, worked previously as well
        vm.createSelectFork("sepolia", 7215400);
        vm.assertFalse(block.prevrandao == 0);

        // Would previously fail with:
        // [FAIL: backend: failed while inspecting; header validation error: `prevrandao` not set; `prevrandao` not set; ] setUp() (gas: 0)
        //
        // Related fix:
        // Moonbeam | Moonbase | Moonriver | MoonbeamDev => {
        //     if env.block.prevrandao.is_none() {
        //         // <https://github.com/foundry-rs/foundry/issues/4232>
        //         env.block.prevrandao = Some(B256::random());
        //     }
        // }
        //
        // Note: public RPC node used for `moonbeam` discards state quickly so we need to fork against the latest block
        vm.createSelectFork("moonbeam");
        vm.assertFalse(block.prevrandao == 0);
    }
}
