// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/10957
contract Issue10957Test is Test {
    function testCreateSelectForkBlockNumber() public {
        // Transaction hash from mainnet <https://etherscan.io/tx/0x2e175897d19307c664815129720c8ac3581da6cb92e4cce923996dd59fbb6ffc>
        bytes32 txHash = 0x2e175897d19307c664815129720c8ac3581da6cb92e4cce923996dd59fbb6ffc;

        // Expected block number for this transaction
        uint256 expectedBlockNumber = 22875105;

        // Create fork at the transaction
        uint256 forkId = vm.createSelectFork("mainnet", txHash);

        // Get the current block number
        uint256 currentBlock = vm.getBlockNumber();

        // The fork should be at the transaction's block, not one block behind
        assertEq(currentBlock, expectedBlockNumber, "Fork should be at the transaction's block number");
    }
}
