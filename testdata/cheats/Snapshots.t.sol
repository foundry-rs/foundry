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

//    function testStore() public {
//        assertEq(store.slot0, 10, "initial value for slot 0 is incorrect");
//        assertEq(store.slot1, 20, "initial value for slot 1 is incorrect");
//    }

    function testSnapshot() public {
        uint256 snapshot = cheats.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

//        assertEq(store.slot0, 300);
//        assertEq(store.slot1, 400);

        cheats.revertTo(snapshot);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
    }

}