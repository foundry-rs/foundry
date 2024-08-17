// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/4640
contract Issue4640Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testArbitrumBlockNumber() public {
        // <https://arbiscan.io/block/75219831>
        vm.createSelectFork("arbitrum", 75219831);
        // L1 block number
        assertEq(block.number, 16939475);
    }
}
