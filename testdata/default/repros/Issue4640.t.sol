// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/4640
contract Issue4640Test is Test {
    function testArbitrumBlockNumber() public {
        // <https://arbiscan.io/block/75219831>
        vm.createSelectFork("arbitrum", 75219831);
        // L1 block number
        assertEq(block.number, 16939475);
    }
}
