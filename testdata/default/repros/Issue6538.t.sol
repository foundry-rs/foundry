// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6538
contract Issue6538Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function test_transact() public {
        bytes32 lastHash = 0xdbdce1d5c14a6ca17f0e527ab762589d6a73f68697606ae0bb90df7ac9ec5087;
        vm.createSelectFork("mainnet", lastHash);
        bytes32 txhash = 0xadbe5cf9269a001d50990d0c29075b402bcc3a0b0f3258821881621b787b35c6;
        vm.transact(txhash);
    }
}
