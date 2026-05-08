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

contract BlobhashRecorder {
    mapping(uint256 => bytes32) public hash;

    function record() external {
        hash[0] = blobhash(0);
        hash[1] = blobhash(1);
    }
}
