// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

struct Storage {
    uint slot0;
    uint slot1;
}

contract SnapshotTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    Storage store;

    function setUp() public {
        store.slot0 = 10;
        store.slot1 = 20;
    }

    function testSnapshot() public {
        uint256 snapshot = cheats.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        cheats.revertTo(snapshot);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
    }

    // tests that snapshots can also revert changes to `block`
    function testBlockValues() public  {
        uint256 num = block.number;
        uint256 time = block.timestamp;

        uint256 snapshot = cheats.snapshot();

        cheats.warp(1337);
        assertEq(block.timestamp, 1337);

        cheats.roll(99);
        assertEq(block.number, 99);

        assert(cheats.revertTo(snapshot));

        assertEq(block.number, num, "snapshot revert for block.number unsuccessful");
        assertEq(block.timestamp, time, "snapshot revert for block.timestamp unsuccessful");
    }

}