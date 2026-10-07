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

// Snapshots inherited from setUp must be deletable before backend initialization.
contract StateSnapshotDeleteFromSetUpTest is Test {
    uint256 id;

    function setUp() public {
        id = vm.snapshotState();
    }

    function testDeleteStateSnapshotTakenInSetUpAsFirstCall() public {
        // Keep deletion as the first mutating cheatcode.
        assertTrue(vm.deleteStateSnapshot(id));
        assert(!vm.revertToState(id));
    }

    function testDeleteStateSnapshotsTakenInSetUpAsFirstCall() public {
        vm.deleteStateSnapshots();
        assert(!vm.revertToState(id));
    }

    function testFuzz_DeleteStateSnapshotTakenInSetUp(uint256) public {
        // Each fuzz run inherits the same setup snapshot.
        assertTrue(vm.deleteStateSnapshot(id));
    }
}

/// forge-config: default.isolate = true
contract StateSnapshotIsolationTest is Test {
    uint256 value;

    function testRevertFromIsolatedCallRestoresAbsentState() public {
        address target = address(0xBEEF);
        uint256 snapshotId = vm.snapshotState();
        value = 2;
        vm.deal(target, 5 ether);

        assertTrue(this.restore(snapshotId));

        assertEq(value, 0);
        assertEq(target.balance, 0);
    }

    function restore(uint256 snapshotId) external returns (bool) {
        return vm.revertToState(snapshotId);
    }
}

contract StateSnapshotNestedRevertTest is Test {
    uint256 value;

    function testRevertFromNestedCallKeepsCallDepth() public {
        uint256 snapshotId = vm.snapshotState();
        value = 2;

        assertTrue(this.restore(snapshotId));

        assertEq(value, 0);
    }

    function restore(uint256 snapshotId) external returns (bool) {
        return vm.revertToState(snapshotId);
    }
}

contract NestedRestoreStore {
    uint256 public value;
    uint256 public marker;
    mapping(uint256 => uint256) public slots;

    function set(uint256 newValue) external {
        value = newValue;
    }

    function mark(uint256 newMarker) external {
        marker = newMarker;
    }

    function fill(uint256 count) external {
        for (uint256 i = 1; i <= count; i++) {
            slots[i] = i;
        }
    }
}

contract NestedRestoreHelper {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function restoreFillRevert(uint256 snapshotId, NestedRestoreStore store, uint256 count) external {
        require(vm.revertToState(snapshotId), "restore failed");
        store.fill(count);
        store.set(42);
        revert("restore reverted");
    }

    function writeRestoreRevert(uint256 snapshotId, NestedRestoreStore store) external {
        store.set(50);
        store.fill(100);
        require(vm.revertToState(snapshotId), "restore failed");
        store.set(42);
        revert("restore reverted");
    }

    function restoreAndDeleteRevert(uint256 snapshotId, NestedRestoreStore store) external {
        require(vm.revertToStateAndDelete(snapshotId), "restore failed");
        store.set(42);
        revert("restore reverted");
    }

    function restoreTwiceRevert(uint256 snapshotId, NestedRestoreStore store) external {
        require(vm.revertToState(snapshotId), "restore failed");
        store.fill(3);
        require(vm.revertToState(snapshotId), "second restore failed");
        store.set(42);
        revert("restore reverted");
    }

    function restoreSet(uint256 snapshotId, NestedRestoreStore store, uint256 newValue) external {
        require(vm.revertToState(snapshotId), "restore failed");
        store.set(newValue);
    }

    function nestedRestoresThenRevert(uint256 snapshotId, NestedRestoreStore store, uint256 count) external {
        store.set(50);
        store.fill(100);
        for (uint256 i; i < count; i++) {
            this.restoreSet(snapshotId, store, 42 + i);
            store.fill(100);
        }
        revert("outer reverted");
    }

    function catchNestedRestoreRevert(uint256 snapshotId, NestedRestoreStore store) external {
        try this.restoreFillRevert(snapshotId, store, 100) {
            revert("unexpected success");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("restore reverted"), reason);
        }
        store.set(store.value() + 100);
    }

    function restoreThenCatchRevert(uint256 snapshotId, NestedRestoreStore store) external {
        require(vm.revertToState(snapshotId), "restore failed");
        store.set(5);
        try this.setAndRevert(store) {
            revert("unexpected success");
        } catch Error(string memory reason) {
            require(keccak256(bytes(reason)) == keccak256("inner reverted"), reason);
        }
    }

    function setAndRevert(NestedRestoreStore store) external {
        store.fill(100);
        store.set(77);
        revert("inner reverted");
    }

    function broadcastThenRevert(bytes calldata rawTx, NestedRestoreStore store) external {
        store.set(2);
        vm.broadcastRawTransaction(rawTx);
        require(store.value() == 2, "transaction restore failed");
        require(store.marker() == 99, "transaction did not finish");
        store.set(3);
        revert("outer reverted");
    }
}

contract NestedRestoreConstructor {
    constructor(uint256 snapshotId, NestedRestoreStore store) {
        Vm vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
        require(vm.revertToState(snapshotId), "restore failed");
        store.set(42);
        revert("constructor reverted");
    }
}

/// A failing frame reinstates the live journal replaced by a snapshot restoration and unwinds
/// its journaled writes, regardless of call isolation.
abstract contract NestedRestoreFrameRevertBase is Test {
    uint256 constant MARKER = 7;

    NestedRestoreStore store;
    NestedRestoreHelper helper;

    function setUp() public {
        store = new NestedRestoreStore();
        helper = new NestedRestoreHelper();
    }

    /// Takes a snapshot, then writes `MARKER` and `tail` values that only a restoration clears.
    function prepare(uint256 tail) internal returns (uint256 snapshotId) {
        store.set(1);
        snapshotId = vm.snapshotState();
        store.mark(MARKER);
        for (uint256 i = 2; i < 2 + tail; i++) {
            store.set(i);
        }
    }

    function assertUndone(uint256 tail, uint256 count) internal {
        assertEq(store.marker(), MARKER);
        assertEq(store.value(), tail + 1);
        assertEq(store.slots(1), 0);
        assertEq(store.slots(count), 0);
    }

    function expectHelperRevert(bytes memory data, string memory reason) internal {
        (bool success, bytes memory output) = address(helper).call(data);
        assertTrue(!success);
        assertEq(output, abi.encodeWithSignature("Error(string)", reason));
    }

    function revertedRestore(uint256 tail, uint256 count) internal {
        uint256 snapshotId = prepare(tail);
        expectHelperRevert(abi.encodeCall(helper.restoreFillRevert, (snapshotId, store, count)), "restore reverted");
        assertUndone(tail, count);
    }

    function testRevertedRestoreLongWrites() public {
        revertedRestore(0, 100);
    }

    function testRevertedRestoreLongTailShortWrites() public {
        revertedRestore(20, 1);
    }

    function testRevertedRestoreUndoesEarlierWrites() public {
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.writeRestoreRevert, (snapshotId, store)), "restore reverted");
        assertUndone(20, 100);
    }

    function testExpectedRevertRestoreIsUndone() public {
        uint256 snapshotId = prepare(20);
        vm.expectRevert("restore reverted");
        helper.restoreFillRevert(snapshotId, store, 100);
        assertUndone(20, 100);
    }

    function testRevertedRestoreTwiceIsUndone() public {
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.restoreTwiceRevert, (snapshotId, store)), "restore reverted");
        assertUndone(20, 3);
    }

    function testRevertedRestoreKeepsSnapshot() public {
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.restoreFillRevert, (snapshotId, store, 100)), "restore reverted");
        assertUndone(20, 100);

        assertTrue(vm.revertToState(snapshotId));
        assertEq(store.marker(), 0);
        assertEq(store.value(), 1);
        assertEq(vm.snapshotState(), snapshotId + 1);
    }

    function testRevertedRestoreAndDeleteStillDeletes() public {
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.restoreAndDeleteRevert, (snapshotId, store)), "restore reverted");
        assertUndone(20, 1);

        assertTrue(!vm.revertToState(snapshotId));
        assertEq(store.marker(), MARKER);
    }

    function testRestoreInSucceedingCallIsKept() public {
        uint256 snapshotId = prepare(20);
        helper.restoreSet(snapshotId, store, 5);
        assertEq(store.marker(), 0);
        assertEq(store.value(), 5);
    }

    function testRevertedOuterCallUndoesSiblingRestores() public {
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.nestedRestoresThenRevert, (snapshotId, store, 3)), "outer reverted");
        assertUndone(20, 100);
    }

    function testSiblingRestoresAreKept() public {
        uint256 snapshotId = prepare(20);
        for (uint256 i; i < 3; i++) {
            helper.restoreSet(snapshotId, store, 42 + i);
            assertEq(store.marker(), 0);
            store.mark(MARKER + i);
        }
        assertEq(store.marker(), MARKER + 2);
        assertEq(store.value(), 44);
    }

    function testCaughtNestedRestoreRevert() public {
        uint256 snapshotId = prepare(20);
        helper.catchNestedRestoreRevert(snapshotId, store);
        assertEq(store.marker(), MARKER);
        assertEq(store.value(), 121);
        assertEq(store.slots(100), 0);
    }

    function testRestoreSurvivesRevertedInnerCall() public {
        uint256 snapshotId = prepare(20);
        helper.restoreThenCatchRevert(snapshotId, store);
        assertEq(store.marker(), 0);
        assertEq(store.value(), 5);
        assertEq(store.slots(100), 0);
    }

    function testDirectRestoreSurvivesRevertedCall() public {
        uint256 snapshotId = prepare(20);
        assertTrue(vm.revertToState(snapshotId));
        assertEq(store.marker(), 0);
        store.set(5);
        expectHelperRevert(abi.encodeCall(helper.setAndRevert, (store)), "inner reverted");
        assertEq(store.marker(), 0);
        assertEq(store.value(), 5);
        assertEq(store.slots(100), 0);
    }

    function testRevertedConstructorRestoreIsUndone() public {
        uint256 snapshotId = prepare(20);
        uint64 nonce = vm.getNonce(address(this));
        vm.expectRevert("constructor reverted");
        new NestedRestoreConstructor(snapshotId, store);
        assertUndone(20, 1);
        assertEq(vm.getNonce(address(this)), nonce + 1);
    }

    function testRawTransactionRestoreDoesNotEscape() public {
        store.set(1);
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.snapshotWriteRestore, ()));
        expectHelperRevert(abi.encodeCall(helper.broadcastThenRevert, (rawTx, store)), "outer reverted");
        assertEq(store.value(), 1);
    }

    function snapshotWriteRestore() external {
        uint256 snapshotId = vm.snapshotState();
        store.set(9);
        require(vm.revertToState(snapshotId), "restore failed");
        require(store.value() == 2, "unexpected restored value");
        store.mark(99);
    }

    function testRawTransactionCaughtRestoreRevert() public {
        vm.broadcastRawTransaction(signedTransaction(abi.encodeCall(this.caughtTransactionRestore, ())));
        assertEq(store.marker(), MARKER);
        assertEq(store.value(), 121);
        assertEq(store.slots(1), 0);
    }

    function testExecuteTransactionCaughtRestoreRevert() public {
        vm.executeTransaction(signedTransaction(abi.encodeCall(this.caughtTransactionRestore, ())));
        assertEq(store.marker(), MARKER);
        assertEq(store.value(), 121);
        assertEq(store.slots(1), 0);
    }

    function caughtTransactionRestore() external {
        uint256 snapshotId = prepare(0);
        expectHelperRevert(abi.encodeCall(helper.restoreFillRevert, (snapshotId, store, 1)), "restore reverted");
        assertUndone(0, 1);
        store.set(121);
    }

    function signedTransaction(bytes memory data) internal returns (bytes memory) {
        uint256 privateKey = 1;
        vm.chainId(1);
        vm.deal(vm.addr(privateKey), 1 ether);

        bytes[] memory unsigned = new bytes[](9);
        unsigned[1] = hex"01";
        unsigned[2] = hex"030d40";
        unsigned[3] = abi.encodePacked(address(this));
        unsigned[5] = data;
        unsigned[6] = hex"01";

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privateKey, keccak256(vm.toRlp(unsigned)));
        unsigned[6] = abi.encodePacked(v + 10);
        unsigned[7] = trimLeadingZeros(r);
        unsigned[8] = trimLeadingZeros(s);
        return vm.toRlp(unsigned);
    }

    function trimLeadingZeros(bytes32 word) internal pure returns (bytes memory out) {
        uint256 offset;
        while (offset < 32 && word[offset] == bytes1(0)) {
            offset++;
        }
        out = new bytes(32 - offset);
        for (uint256 i; i < out.length; i++) {
            out[i] = word[offset + i];
        }
    }
}

/// forge-config: default.isolate = true
contract NestedRestoreFrameRevertIsolatedTest is NestedRestoreFrameRevertBase {}

/// forge-config: default.isolate = false
contract NestedRestoreFrameRevertNonIsolatedTest is NestedRestoreFrameRevertBase {}

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
