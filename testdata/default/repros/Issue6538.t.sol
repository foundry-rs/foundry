// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/6538
contract Issue6538Test is Test {
    function test_transact() public {
        bytes32 lastHash = 0x4b70ca8c5a0990b43df3064372d424d46efa41dfaab961754b86c5afb2df4f61;
        vm.createSelectFork("mainnet", lastHash);
        bytes32 txhash = 0x7dcff74771babf9c23363c4228e55a27f50224d4596b1ba6608b0b45712f94ba;
        vm.transact(txhash);
    }
}
