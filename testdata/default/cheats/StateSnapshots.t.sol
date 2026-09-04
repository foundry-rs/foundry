// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

struct Storage {
    uint256 slot0;
    uint256 slot1;
}

contract StateSnapshotTest is Test {
    Storage store;

    function setUp() public {
        store.slot0 = 10;
        store.slot1 = 20;
    }

    function testStateSnapshot() public {
        uint256 snapshotId = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertToState(snapshotId);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
    }

    function testStateSnapshotRevertDelete() public {
        uint256 snapshotId = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertToStateAndDelete(snapshotId);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
        // nothing to revert to anymore
        assert(!vm.revertToState(snapshotId));
    }

    function testStateSnapshotDelete() public {
        uint256 snapshotId = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        vm.deleteStateSnapshot(snapshotId);
        // nothing to revert to anymore
        assert(!vm.revertToState(snapshotId));
    }

    function testStateSnapshotDeleteAll() public {
        uint256 snapshotId = vm.snapshotState();
        store.slot0 = 300;
        store.slot1 = 400;

        vm.deleteStateSnapshots();
        // nothing to revert to anymore
        assert(!vm.revertToState(snapshotId));
    }

    // <https://github.com/foundry-rs/foundry/issues/6411>
    function testStateSnapshotsMany() public {
        uint256 snapshotId;
        for (uint256 c = 0; c < 10; c++) {
            for (uint256 cc = 0; cc < 10; cc++) {
                snapshotId = vm.snapshotState();
                vm.revertToStateAndDelete(snapshotId);
                assert(!vm.revertToState(snapshotId));
            }
        }
    }

    // tests that snapshots can also revert changes to `block`
    function testBlockValues() public {
        uint256 num = block.number;
        uint256 time = block.timestamp;
        uint256 prevrandao = block.prevrandao;

        uint256 snapshotId = vm.snapshotState();

        vm.warp(1337);
        assertEq(block.timestamp, 1337);

        vm.roll(99);
        assertEq(block.number, 99);

        vm.prevrandao(uint256(123));
        assertEq(block.prevrandao, 123);

        assert(vm.revertToState(snapshotId));

        assertEq(block.number, num, "snapshot revert for block.number unsuccessful");
        assertEq(block.timestamp, time, "snapshot revert for block.timestamp unsuccessful");
        assertEq(block.prevrandao, prevrandao, "snapshot revert for block.prevrandao unsuccessful");
    }
}

// A snapshot taken in `setUp()` must be deletable as the very FIRST cheatcode call of a test
// run, not only after some other mutating cheatcode has already run in that same call. Each
// test/fuzz run executes as a fresh, non-committing call over the post-`setUp()` state, and the
// deletion must not depend on incidental prior mutating-cheatcode history within that run.
contract StateSnapshotDeleteFromSetUpTest is Test {
    uint256 id;

    function setUp() public {
        id = vm.snapshotState();
    }

    function testDeleteStateSnapshotTakenInSetUpAsFirstCall() public {
        // Must be the first cheatcode call in this test body - anything mutating before it
        // would incidentally unmask the bug by forcing backend initialization early.
        assertTrue(vm.deleteStateSnapshot(id));
        assert(!vm.revertToState(id));
    }

    function testDeleteStateSnapshotsTakenInSetUpAsFirstCall() public {
        vm.deleteStateSnapshots();
        assert(!vm.revertToState(id));
    }

    function testFuzz_DeleteStateSnapshotTakenInSetUp(uint256) public {
        // Every fuzz run gets its own fresh, non-committing execution over the post-`setUp()`
        // state, so this must pass identically on every run, not just after the first.
        assertTrue(vm.deleteStateSnapshot(id));
    }
}

// TODO: remove this test suite once `snapshot*` has been deprecated in favor of `snapshotState*`.
contract DeprecatedStateSnapshotTest is Test {
    Storage store;

    function setUp() public {
        store.slot0 = 10;
        store.slot1 = 20;
    }

    function testSnapshotState() public {
        uint256 snapshotId = vm.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertTo(snapshotId);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
    }

    function testSnapshotStateRevertDelete() public {
        uint256 snapshotId = vm.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

        assertEq(store.slot0, 300);
        assertEq(store.slot1, 400);

        vm.revertToAndDelete(snapshotId);
        assertEq(store.slot0, 10, "snapshot revert for slot 0 unsuccessful");
        assertEq(store.slot1, 20, "snapshot revert for slot 1 unsuccessful");
        // nothing to revert to anymore
        assert(!vm.revertTo(snapshotId));
    }

    function testSnapshotStateDelete() public {
        uint256 snapshotId = vm.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

        vm.deleteSnapshot(snapshotId);
        // nothing to revert to anymore
        assert(!vm.revertTo(snapshotId));
    }

    function testSnapshotStateDeleteAll() public {
        uint256 snapshotId = vm.snapshot();
        store.slot0 = 300;
        store.slot1 = 400;

        vm.deleteSnapshots();
        // nothing to revert to anymore
        assert(!vm.revertTo(snapshotId));
    }

    // <https://github.com/foundry-rs/foundry/issues/6411>
    function testSnapshotStatesMany() public {
        uint256 snapshotId;
        for (uint256 c = 0; c < 10; c++) {
            for (uint256 cc = 0; cc < 10; cc++) {
                snapshotId = vm.snapshot();
                vm.revertToAndDelete(snapshotId);
                assert(!vm.revertTo(snapshotId));
            }
        }
    }

    // tests that snapshots can also revert changes to `block`
    function testBlockValues() public {
        uint256 num = block.number;
        uint256 time = block.timestamp;
        uint256 prevrandao = block.prevrandao;

        uint256 snapshotId = vm.snapshot();

        vm.warp(1337);
        assertEq(block.timestamp, 1337);

        vm.roll(99);
        assertEq(block.number, 99);

        vm.prevrandao(uint256(123));
        assertEq(block.prevrandao, 123);

        assert(vm.revertTo(snapshotId));

        assertEq(block.number, num, "snapshot revert for block.number unsuccessful");
        assertEq(block.timestamp, time, "snapshot revert for block.timestamp unsuccessful");
        assertEq(block.prevrandao, prevrandao, "snapshot revert for block.prevrandao unsuccessful");
    }
}
