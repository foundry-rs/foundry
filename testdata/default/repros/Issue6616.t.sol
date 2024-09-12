// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/6616
contract Issue6616Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testCreateForkRollLatestBlock() public {
        vm.createSelectFork("mainnet");
        uint256 startBlock = block.number;
        // this will create new forks and exit once a new latest block is found
        for (uint256 i; i < 10; i++) {
            vm.sleep(5000);
            vm.createSelectFork("mainnet");
            if (block.number > startBlock) break;
        }
        assertGt(block.number, startBlock);
    }
}
