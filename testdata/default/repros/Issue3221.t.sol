// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/3221
contract Issue3221Test is Test {
    uint256 fork1;
    uint256 fork2;

    function setUp() public {
        fork1 = vm.createFork("mainnet", 20000000);
        fork2 = vm.createFork("avaxTestnet", 12880747);
    }

    function testForkNonce() public {
        address user = address(0xa0Ee7A142d267C1f36714E4a8F75612F20a79720);

        // Loads but doesn't touch
        assertEq(vm.getNonce(user), 0);

        vm.selectFork(fork2);
        assertEq(vm.getNonce(user), 13);
        vm.prank(user);
        new Counter();

        vm.selectFork(fork1);
        assertEq(vm.getNonce(user), 9);
        vm.prank(user);
        new Counter();
    }
}

contract Counter {}
