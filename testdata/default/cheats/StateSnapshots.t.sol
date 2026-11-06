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

    function destroy() external {
        selfdestruct(payable(address(this)));
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

    function restoreWriteRevert(uint256 snapshotId, NestedRestoreStore store) external {
        require(vm.revertToState(snapshotId), "restore failed");
        vm.deal(address(0xa1), 3);
        this.setNonces();
        store.value();
        vm.copyStorage(address(store), address(0xa4));
        vm.cloneAccount(address(store), address(0xa6));
        vm.signAndAttachDelegation(address(store), 0xa8);
        vm.broadcast(address(0xa7));
        store.mark(1);
        require(vm.revertToState(snapshotId), "second restore failed");
        revert("restore reverted");
    }

    function setNonces() external {
        vm.setNonce(address(0xa2), 4);
        vm.setNonceUnsafe(address(0xa3), 5);
        vm.resetNonce(address(0xa5));
    }

    function restoreTransactRevert(uint256 snapshotId, bytes calldata rawTx, bool broadcast) external {
        require(vm.revertToState(snapshotId), "restore failed");
        if (broadcast) {
            vm.broadcastRawTransaction(rawTx);
        } else {
            vm.executeTransaction(rawTx);
        }
        revert("restore reverted");
    }

    function executeAndRevert(bytes calldata rawTx) external {
        vm.executeTransaction(rawTx);
        revert("transaction reverted");
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
    NestedRestoreStore created;
    bytes creationTx;

    function setUp() public {
        store = new NestedRestoreStore();
        helper = new NestedRestoreHelper();
        creationTx = signedTransaction(hex"6007600055600060005360016000f3", true);
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

    /// Cheatcode writes that bypass the journal survive a failing frame that undoes the restoration
    /// they followed, as they survive any other revert.
    function testRevertedRestoreKeepsLaterUnjournaledWrites() public {
        vm.setNonce(address(0xa5), 9);
        uint256 snapshotId = prepare(20);
        vm.store(address(0xa4), bytes32(uint256(5)), bytes32(uint256(99)));
        expectHelperRevert(abi.encodeCall(helper.restoreWriteRevert, (snapshotId, store)), "restore reverted");
        assertUndone(20, 1);
        assertEq(address(0xa1).balance, 3);
        assertEq(vm.getNonce(address(0xa2)), 4);
        assertEq(vm.getNonce(address(0xa3)), 5);
        assertEq(vm.getNonce(address(0xa5)), 0);
        assertEq(vm.load(address(0xa4), 0), bytes32(uint256(1)));
        assertEq(vm.load(address(0xa4), bytes32(uint256(5))), bytes32(0));
        assertEq(vm.getNonce(address(0xa6)), 1);
        assertEq(vm.load(address(0xa6), 0), bytes32(uint256(1)));
        assertEq(address(0xa6).code.length, 0);
        assertEq(vm.getNonce(address(0xa7)), 1);
        assertEq(vm.getNonce(vm.addr(0xa8)), 1);
    }

    function testRevertedRestoreKeepsLaterExecutedTransaction() public {
        revertedRestoreTransaction(false);
    }

    function testRevertedRestoreKeepsLaterRawTransaction() public {
        revertedRestoreTransaction(true);
    }

    function revertedRestoreTransaction(bool broadcast) internal {
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.markStore, (99)));
        uint256 snapshotId = prepare(20);
        expectHelperRevert(
            abi.encodeCall(helper.restoreTransactRevert, (snapshotId, rawTx, broadcast)), "restore reverted"
        );
        assertEq(store.marker(), 99);
        assertEq(store.value(), 21);
        assertEq(vm.getNonce(vm.addr(1)), 1);
    }

    function markStore(uint256 newMarker) external {
        store.mark(newMarker);
    }

    /// An account created by the transaction stays created in the current transaction, so a
    /// `selfdestruct` deletes it and burns its balance.
    function testRevertedRestoreKeepsLaterTransactionCreation() public {
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.createStore, ()));
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.restoreTransactRevert, (snapshotId, rawTx, false)), "restore reverted");
        vm.deal(address(created), 3);
        created.destroy();
        assertEq(address(created).balance, 0);
    }

    function createStore() external {
        created = new NestedRestoreStore();
    }

    /// An account destroyed by a committed transaction stays deleted.
    function testRevertedRestoreKeepsLaterTransactionDestruction() public {
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.createAndDestroyStore, ()));
        uint256 snapshotId = prepare(20);
        expectHelperRevert(abi.encodeCall(helper.restoreTransactRevert, (snapshotId, rawTx, true)), "restore reverted");
        assertEq(address(created).code.length, 0);
    }

    function createAndDestroyStore() external {
        created = new NestedRestoreStore();
        created.destroy();
    }

    function testRevertedRestoreClearsCommittedCreationStorage() public {
        // The initcode writes slot 0 to 7 and returns a single STOP instruction.
        bytes memory rawTx = signedTransaction(hex"6007600055600060005360016000f3", true);
        address target = vm.computeCreateAddress(vm.addr(1), 0);
        uint256 snapshotId = vm.snapshotState();
        vm.store(target, bytes32(uint256(1)), bytes32(uint256(99)));
        expectHelperRevert(abi.encodeCall(helper.restoreTransactRevert, (snapshotId, rawTx, true)), "restore reverted");
        assertEq(target.code, hex"00");
        assertEq(vm.load(target, bytes32(0)), bytes32(uint256(7)));
        assertEq(vm.load(target, bytes32(uint256(1))), bytes32(0));
    }

    function testRevertedRestoreKeepsExplicitUnchangedTransactionNonce() public {
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.setTransactionNonce, ()));
        vm.setNonceUnsafe(address(0xa9), 5);
        uint256 snapshotId = vm.snapshotState();
        vm.setNonceUnsafe(address(0xa9), 9);
        expectHelperRevert(abi.encodeCall(helper.restoreTransactRevert, (snapshotId, rawTx, false)), "restore reverted");
        assertEq(vm.getNonce(address(0xa9)), 5);
    }

    function testRevertedRestoreKeepsCommittedCreationNonce() public {
        // Fund and sign in setUp so the sender is absent from the snapshot's journal.
        uint256 snapshotId = vm.snapshotState();
        expectHelperRevert(
            abi.encodeCall(helper.restoreTransactRevert, (snapshotId, creationTx, true)), "restore reverted"
        );
        assertEq(vm.getNonce(vm.addr(1)), 1);
    }

    function testRevertedTransactionKeepsExplicitNonce() public {
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.setTransactionNonce, ()));
        vm.setNonceUnsafe(address(0xa9), 9);
        expectHelperRevert(abi.encodeCall(helper.executeAndRevert, (rawTx)), "transaction reverted");
        assertEq(vm.getNonce(address(0xa9)), 5);
    }

    function setTransactionNonce() external {
        vm.setNonceUnsafe(address(0xa9), 5);
    }

    function testRevertedRestoreKeepsSavedOnlyExecutedRecipientBalance() public {
        revertedRestoreSavedOnlyRecipient(false);
    }

    function testRevertedRestoreKeepsSavedOnlyRawRecipientBalance() public {
        revertedRestoreSavedOnlyRecipient(true);
    }

    function revertedRestoreSavedOnlyRecipient(bool broadcast) internal {
        bytes memory rawTx = signedTransaction(abi.encodeCall(this.payFreshRecipient, ()));
        vm.deal(address(this), 1 ether);
        uint256 snapshotId = vm.snapshotState();
        // Load the recipient only into the journal that the failing restoration reinstates.
        assertEq(vm.getNonce(address(0xb0b0)), 0);
        expectHelperRevert(
            abi.encodeCall(helper.restoreTransactRevert, (snapshotId, rawTx, broadcast)), "restore reverted"
        );
        assertEq(address(0xb0b0).balance, 1);
    }

    function payFreshRecipient() external {
        payable(address(0xb0b0)).transfer(1);
    }

    function signedTransaction(bytes memory data) internal returns (bytes memory) {
        return signedTransaction(data, false);
    }

    function signedTransaction(bytes memory data, bool create) internal returns (bytes memory) {
        uint256 privateKey = 1;
        vm.chainId(1);
        vm.deal(vm.addr(privateKey), 1 ether);

        bytes[] memory unsigned = new bytes[](9);
        unsigned[1] = hex"01";
        unsigned[2] = hex"030d40";
        if (!create) unsigned[3] = abi.encodePacked(address(this));
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
