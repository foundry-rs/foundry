// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";

// https://github.com/foundry-rs/foundry/issues/2623
contract Issue2623Test is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    function testRollFork() public {
        uint256 fork = vm.createFork("rpcAlias", 10);
        vm.selectFork(fork);

        assertEq(block.number, 10);
        assertEq(block.timestamp, 1438270128);

        vm.rollFork(11);

        assertEq(block.number, 11);
        assertEq(block.timestamp, 1438270136);
    }
}
