// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/5929
contract Issue5929Test is Test {
    function test_transact_not_working() public {
        vm.createSelectFork("mainnet", 21134547);
        // https://etherscan.io/tx/0x96a129768ec66fd7d65114bf182f4e173bf0b73a44219adaf71f01381a3d0143
        vm.transact(hex"7dcff74771babf9c23363c4228e55a27f50224d4596b1ba6608b0b45712f94ba");
    }
}
