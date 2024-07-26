// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3221
contract Issue3221Test is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    uint256 fork1;
    uint256 fork2;

    function setUp() public {
        fork1 = vm.createFork("sepolia", 5565573);
        fork2 = vm.createFork("avaxTestnet", 12880747);
    }

    function testForkNonce() public {
        address user = address(0xF0959944122fb1ed4CfaBA645eA06EED30427BAA);

        // Loads but doesn't touch
        assertEq(vm.getNonce(user), 0);

        vm.selectFork(fork2);
        assertEq(vm.getNonce(user), 3);
        vm.prank(user);
        new Counter();

        vm.selectFork(fork1);
        assertEq(vm.getNonce(user), 1);
        vm.prank(user);
        new Counter();
    }
}

contract Counter {}
