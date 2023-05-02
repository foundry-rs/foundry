// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract Victim {
    function assertSender(address sender) external {
        require(msg.sender == sender, "sender was not pranked");
    }
}

contract CheatsSetupTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);
    Victim victim;

    function setUp() public {
        victim = new Victim();

        cheats.warp(10);
        cheats.chainId(99);
        cheats.roll(100);
        cheats.fee(1000);
        cheats.prevrandao(bytes32(uint256(10000)));
        cheats.startPrank(address(1337));
    }

    function testCheatEnvironment() public {
        assertEq(block.timestamp, 10, "block timestamp was not persisted from setup");
        assertEq(block.number, 100, "block number was not persisted from setup");
        assertEq(block.basefee, 1000, "basefee was not persisted from setup");
        assertEq(block.prevrandao, 10000, "prevrandao was not persisted from setup");
        assertEq(block.chainid, 99, "chainid was not persisted from setup");
        victim.assertSender(address(1337));
    }
}
