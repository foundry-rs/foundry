// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3192
contract Issue3192Test is Test {
    uint256 fork1;
    uint256 fork2;

    function setUp() public {
        fork1 = vm.createFork("mainnet", 7475589);
        fork2 = vm.createFork("mainnet", 12880747);
        vm.selectFork(fork1);
    }

    function testForkSwapSelect() public {
        assertEq(fork1, vm.activeFork());
        vm.selectFork(fork2);
    }
}
