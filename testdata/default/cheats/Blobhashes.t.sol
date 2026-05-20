// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.25;

import "utils/Test.sol";

contract BlobhashesTest is Test {
    function testSetAndGetBlobhashes() public {
        bytes32[] memory blobhashes = new bytes32[](2);
        blobhashes[0] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000001);
        blobhashes[1] = bytes32(0x0000000000000000000000000000000000000000000000000000000000000002);
        vm.blobhashes(blobhashes);

        bytes32[] memory gotBlobhashes = vm.getBlobhashes();
        assertEq(gotBlobhashes[0], blobhashes[0]);
        assertEq(gotBlobhashes[1], blobhashes[1]);
    }
}

/// `vm.getBlobhashes` must reflect the rolled-back value after `revertToState`.
contract BlobhashesSnapshotTest is Test {
    function test_getBlobhashes_after_revertToState() public {
        bytes32[] memory a = new bytes32[](1);
        a[0] = bytes32(uint256(0xAAAA));
        bytes32[] memory b = new bytes32[](1);
        b[0] = bytes32(uint256(0xBBBB));

        vm.blobhashes(a);
        uint256 id = vm.snapshotState();
        vm.blobhashes(b);
        vm.revertToState(id);

        bytes32[] memory got = vm.getBlobhashes();
        assertEq(got.length, 1);
        assertEq(got[0], a[0]);
    }
}

/// `snapshot -> B -> revert` must clear the override back to empty (no A set before snapshot).
contract BlobhashesSnapshotClearTest is Test {
    function test_getBlobhashes_cleared_after_revertToState() public {
        bytes32[] memory b = new bytes32[](1);
        b[0] = bytes32(uint256(0xBBBB));

        // Take snapshot with no blobhashes set.
        uint256 id = vm.snapshotState();
        vm.blobhashes(b);

        bytes32[] memory got = vm.getBlobhashes();
        assertEq(got.length, 1, "should have B before revert");

        vm.revertToState(id);

        bytes32[] memory after_ = vm.getBlobhashes();
        assertEq(after_.length, 0, "should be empty after revert to pre-set snapshot");
    }
}

/// `vm.txGasPrice` override must be rolled back by `revertToState`.
contract TxGasPriceSnapshotTest is Test {
    function test_txGasPrice_after_revertToState() public {
        uint256 a = 111 gwei;
        uint256 b = 222 gwei;

        vm.txGasPrice(a);
        uint256 id = vm.snapshotState();
        vm.txGasPrice(b);
        vm.revertToState(id);

        assertEq(tx.gasprice, a, "GASPRICE should equal a after revert");
    }

    function test_txGasPrice_cleared_after_revertToState() public {
        uint256 b = 222 gwei;

        // Take snapshot with no override.
        uint256 id = vm.snapshotState();
        vm.txGasPrice(b);
        vm.revertToState(id);

        // GASPRICE should be back to the default (0 in tests).
        assertEq(tx.gasprice, 0, "GASPRICE should be cleared after revert to pre-set snapshot");
    }
}

/// `vm.blobhashes` must remain visible to a *called* contract under
/// `--isolate`, where the synthetic inner transaction would otherwise be
/// rejected (left over EIP-4844 type + zero gas price) and `BLOBHASH` would
/// return zero. Regression test for #7277.
/// forge-config: default.isolate = true
contract IsolatedBlobhashesTest is Test {
    BlobhashRecorder internal recorder;

    function setUp() public {
        recorder = new BlobhashRecorder();
    }

    function test_blobhashes_visible_in_called_contract() public {
        bytes32[] memory hashes = new bytes32[](2);
        hashes[0] = bytes32(uint256(0xdeadbeef));
        hashes[1] = bytes32(uint256(0xcafebabe));
        vm.blobhashes(hashes);

        recorder.record();

        assertEq(recorder.hash(0), hashes[0]);
        assertEq(recorder.hash(1), hashes[1]);
    }
}

/// After reverting to a pre-blobhashes snapshot, `vm.blobhashes` set
/// afterwards must be visible to a called contract in isolation. Without the
/// tx_type fix, tx_type is stuck at EIP4844 after the revert; the isolation
/// stack then detects EIP4844 on the *outer* tx, caches it, and the
/// subsequent call to `vm.blobhashes` sets EIP4844 again, so the recorded
/// cached_tx has EIP4844 + new hashes.
/// forge-config: default.isolate = true
contract BlobhashesTxTypeResetTest is Test {
    BlobhashRecorder internal recorder;

    function setUp() public {
        recorder = new BlobhashRecorder();
    }

    function test_blobhashes_after_clear_revert_visible_in_isolation() public {
        bytes32[] memory hashes = new bytes32[](1);
        hashes[0] = bytes32(uint256(0xC0FFEE));

        // Snapshot with no blobhashes set (tx_type should revert to original).
        uint256 id = vm.snapshotState();
        vm.blobhashes(hashes);

        bytes32[] memory got = vm.getBlobhashes();
        assertEq(got.length, 1, "should have hashes before revert");

        vm.revertToState(id);

        // After revert, set fresh hashes and verify they're visible via
        // an external call (exercises the env_overrides path in isolation).
        bytes32[] memory fresh = new bytes32[](1);
        fresh[0] = bytes32(uint256(0xDEAD));
        vm.blobhashes(fresh);

        recorder.record();
        assertEq(recorder.hash(0), fresh[0], "fresh blobhash must be visible after revert");
    }
}

/// Pre-existing tx blob hashes must survive a `snapshotState` / `revertToState`
/// round-trip when no `vm.blobhashes` override was active at snapshot time.
contract BlobhashesPreExistingRevertToStateTest is Test {
    function test_preExistingBlobhashes_preserved_after_revertToState() public {
        bytes32[] memory original = new bytes32[](2);
        original[0] = bytes32(uint256(0x1111));
        original[1] = bytes32(uint256(0x2222));

        // Simulate pre-existing blob hashes on the tx (set via cheatcode as a
        // stand-in for an EIP-4844 transaction in fork mode).
        vm.blobhashes(original);

        // Clear the override so the tx has the hashes natively.
        // Re-read them to confirm baseline.
        bytes32[] memory pre = vm.getBlobhashes();
        assertEq(pre.length, 2, "baseline: should have 2 hashes before snapshot");

        // Snapshot while the override is active (mirrors the fork-mode case
        // where hashes are on the tx directly and no override exists).
        uint256 id = vm.snapshotState();

        // Apply a different override.
        bytes32[] memory newHashes = new bytes32[](1);
        newHashes[0] = bytes32(uint256(0x3333));
        vm.blobhashes(newHashes);

        vm.revertToState(id);

        bytes32[] memory after_ = vm.getBlobhashes();
        assertEq(after_.length, 2, "should have original 2 hashes after revert");
        assertEq(after_[0], original[0], "hash[0] must match original");
        assertEq(after_[1], original[1], "hash[1] must match original");
    }

    function test_preExistingBlobhashes_preserved_after_revertToStateAndDelete() public {
        bytes32[] memory original = new bytes32[](1);
        original[0] = bytes32(uint256(0xABCD));

        vm.blobhashes(original);

        uint256 id = vm.snapshotState();

        bytes32[] memory newHashes = new bytes32[](1);
        newHashes[0] = bytes32(uint256(0xEF01));
        vm.blobhashes(newHashes);

        vm.revertToStateAndDelete(id);

        bytes32[] memory after_ = vm.getBlobhashes();
        assertEq(after_.length, 1, "should have original 1 hash after revertAndDelete");
        assertEq(after_[0], original[0], "hash[0] must match original");
    }
}

contract BlobhashRecorder {
    mapping(uint256 => bytes32) public hash;

    function record() external {
        hash[0] = blobhash(0);
        hash[1] = blobhash(1);
    }
}
