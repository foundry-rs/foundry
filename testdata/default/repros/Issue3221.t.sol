// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

// https://github.com/foundry-rs/foundry/issues/3221
contract Issue3221Test is DSTest {
    Vm constant VM = Vm(HEVM_ADDRESS);
    uint256 fork1;
    uint256 fork2;

    function setUp() public {
        fork1 = VM.createFork("sepolia", 5565573);
        fork2 = VM.createFork("avaxTestnet", 12880747);
    }

    function testForkNonce() public {
        address user = address(0xF0959944122fb1ed4CfaBA645eA06EED30427BAA);

        // Loads but doesn't touch
        assertEq(VM.getNonce(user), 0);

        VM.selectFork(fork2);
        assertEq(VM.getNonce(user), 3);
        VM.prank(user);
        new Counter();

        VM.selectFork(fork1);
        assertEq(VM.getNonce(user), 1);
        VM.prank(user);
        new Counter();
    }
}

contract Counter {}
