// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract CheatsSetupTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function setUp() public {
      cheats.warp(10);
      cheats.roll(100);
      cheats.fee(1000);
    }

    function testCheatEnvironment() public {
        assertEq(block.timestamp, 10, "block timestamp was not persisted from setup");
        assertEq(block.number, 100, "block number was not persisted from setup");
        assertEq(block.basefee, 1000, "basefee was not persisted from setup");
    }
}
