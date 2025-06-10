// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/2623
contract Issue2623Test is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);

    function testRollFork() public {
        uint256 fork = VM.createFork("mainnet", 10);
        VM.selectFork(fork);

        assertEq(block.number, 10);
        assertEq(block.timestamp, 1438270128);

        VM.rollFork(11);

        assertEq(block.number, 11);
        assertEq(block.timestamp, 1438270136);
    }
}
