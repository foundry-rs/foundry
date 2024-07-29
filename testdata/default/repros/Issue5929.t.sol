// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/5929
contract Issue5929Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_transact_not_working() public {
        vm.createSelectFork("mainnet", 15625301);
        // https://etherscan.io/tx/0x96a129768ec66fd7d65114bf182f4e173bf0b73a44219adaf71f01381a3d0143
        vm.transact(hex"96a129768ec66fd7d65114bf182f4e173bf0b73a44219adaf71f01381a3d0143");
    }
}
