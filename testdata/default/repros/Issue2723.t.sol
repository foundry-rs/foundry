// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/2723
contract Issue2723Test is Test {
    function testRollFork() public {
        address coinbase = 0x0193d941b50d91BE6567c7eE1C0Fe7AF498b4137;

        vm.createSelectFork("mainnet", 9);

        assertEq(block.number, 9);
        assertEq(coinbase.balance, 11250000000000000000);

        vm.rollFork(10);

        assertEq(block.number, 10);
        assertEq(coinbase.balance, 16250000000000000000);
    }
}
