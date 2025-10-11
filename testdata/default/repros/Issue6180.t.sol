// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/6180
contract Issue6180Test is Test {
    function test_timebug() external {
        uint256 start = block.timestamp;
        uint256 count = 4;
        uint256 duration = 15;
        for (uint256 i; i < count; i++) {
            vm.warp(block.timestamp + duration);
        }

        uint256 end = block.timestamp;
        assertEq(end, start + count * duration);
        assertEq(end - start, count * duration);
    }
}
