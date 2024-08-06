// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

struct Storage {
    uint256 slot0;
    uint256 slot1;
}

contract StateSnapshotTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    Storage store;

    function setUp() public {
        store.slot0 = 10;
        store.slot1 = 20;
    }

    function testSnapshotState() public {
        uint256 snapshot = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertToState(snapshot);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
    }

    function testSnapshotStateRevertDelete() public {
        uint256 snapshot = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertToStateAndDelete(snapshot);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
        // nothing to revert to anymore
        assert(!vm.revertToState(snapshot));
    }

    function testSnapshotStateDelete() public {
        uint256 snapshot = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        vm.deleteSnapshot(snapshot);
        // nothing to revert to anymore
        assert(!vm.revertToState(snapshot));
    }

    function testSnapshotStateDeleteAll() public {
        uint256 snapshot = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        vm.deleteSnapshots();
        // nothing to revert to anymore
        assert(!vm.revertToState(snapshot));
    }

    // <https://github.com/foundry-rs/foundry/issues/6411>
    function testSnapshotStatesMany() public {
        uint256 preState;
        for (uint256 c = 0; c < 10; c++) {
            for (uint256 cc = 0; cc < 10; cc++) {
                preState = vm.snapshotState();
                vm.revertToStateAndDelete(preState);
                assert(!vm.revertToState(preState));
            }
        }
    }

    // tests that snapshots can also revert changes to `block`
    function testBlockValues() public {
        uint256 num = block.number;
        uint256 time = block.timestamp;
        uint256 prevrandao = block.prevrandao;

        uint256 snapshot = vm.snapshotState();

        vm.warp(1337);
        assertEq(block.timestamp, 1337);

        vm.roll(99);
        assertEq(block.number, 99);

        vm.prevrandao(uint256(123));
        assertEq(block.prevrandao, 123);

        assert(vm.revertToState(snapshot));

        assertEq(
            block.number,
            num,
            "snapshot revert for block.number unsuccessful"
        );
        assertEq(
            block.timestamp,
            time,
            "snapshot revert for block.timestamp unsuccessful"
        );
        assertEq(
            block.prevrandao,
            prevrandao,
            "snapshot revert for block.prevrandao unsuccessful"
        );
    }
}