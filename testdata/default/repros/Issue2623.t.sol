// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/2623
contract Issue2623Test is Test {
    function testRollFork() public {
        uint256 fork = vm.createFork("mainnet", 10);
        vm.selectFork(fork);

        assertEq(block.number, 10);
        assertEq(block.timestamp, 1438270128);

        vm.rollFork(11);

        assertEq(block.number, 11);
        assertEq(block.timestamp, 1438270136);
    }
}
