// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;
import "utils/Test.sol";

contract CapsuleStore {
    uint256 public value;

    function set(uint256 v) external {
        value = v;
    }
}

contract CapsuleHelper is Test {
    event Observed(uint256 indexed stage);

    function restore(uint256 id, bool remove, bool halt) external {
        emit Observed(2);
        bool ok = remove ? vm.revertToStateAndDelete(id) : vm.revertToState(id);
        require(ok, "restore failed");
        emit Observed(3);
        if (halt) {
            assembly { invalid() }
        }
        revert("helper reverted");
    }
}

/// forge-config: default.isolate = true
contract SnapshotCapsulePolicyTest is Test {
    event Observed(uint256 indexed stage);

    function testRevertedRestoreEnvironmentPersists() public {
        checkEnvironment(false);
    }

    function testHaltedRestoreEnvironmentPersists() public {
        checkEnvironment(true);
    }

    function checkEnvironment(bool halt) internal {
        CapsuleHelper helper = new CapsuleHelper();
        CapsuleStore store = new CapsuleStore();
        vm.fee(100);
        vm.chainId(100);
        vm.roll(100);
        vm.warp(100);
        store.set(1);
        uint256 id = vm.snapshotState();
        vm.fee(200);
        vm.chainId(200);
        vm.roll(200);
        vm.warp(200);
        store.set(2);
        (bool ok, bytes memory data) = address(helper).call(abi.encodeCall(helper.restore, (id, false, halt)));
        vm.assertFalse(ok);
        assertEq(data, halt ? bytes("") : abi.encodeWithSignature("Error(string)", "helper reverted"));
        assertEq(store.value(), 2);
        assertEq(block.basefee, 100);
        assertEq(block.chainid, 100);
        assertEq(block.number, 100);
        assertEq(block.timestamp, 100);
    }

    function testHaltedRestoreAndDeleteStillDeletes() public {
        CapsuleHelper helper = new CapsuleHelper();
        uint256 id = vm.snapshotState();
        (bool ok, bytes memory data) = address(helper).call(abi.encodeCall(helper.restore, (id, true, true)));
        vm.assertFalse(ok);
        assertEq(data, "");
        vm.assertFalse(vm.revertToState(id));
    }

    function testRevertedRestoreRecordedLogsRemainOrdered() public {
        checkLogs(false);
    }

    function testHaltedRestoreRecordedLogsRemainOrdered() public {
        checkLogs(true);
    }

    function checkLogs(bool halt) internal {
        CapsuleHelper helper = new CapsuleHelper();
        uint256 id = vm.snapshotState();
        vm.recordLogs();
        emit Observed(1);
        (bool ok, bytes memory data) = address(helper).call(abi.encodeCall(helper.restore, (id, false, halt)));
        vm.assertFalse(ok);
        assertEq(data, halt ? bytes("") : abi.encodeWithSignature("Error(string)", "helper reverted"));
        emit Observed(4);
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertEq(logs.length, 4);
        for (uint256 i; i < 4; i++) {
            assertEq(logs[i].topics.length, 2);
            assertEq(logs[i].topics[0], keccak256("Observed(uint256)"));
            assertEq(uint256(logs[i].topics[1]), i + 1);
            assertEq(logs[i].emitter, i == 1 || i == 2 ? address(helper) : address(this));
        }
    }
}

/// forge-config: default.isolate = false
contract SnapshotCapsulePolicyNonIsolatedTest is SnapshotCapsulePolicyTest {}
