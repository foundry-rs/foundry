// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/4640
contract Issue4640Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function testArbitrumBlockNumber() public {
        // <https://arbiscan.io/block/75219831>
        vm.createSelectFork("https://rpc.ankr.com/arbitrum", 75219831);
        // L1 block number
        assertEq(block.number, 16939475);
    }
}
