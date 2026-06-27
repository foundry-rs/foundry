// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Target {
    uint256 public slot0;

    function expandMemory(uint256 n) public pure returns (uint256) {
        uint256[] memory arr = new uint256[](n);

        for (uint256 i = 0; i < n; i++) {
            arr[i] = i;
        }

        return arr.length;
    }

    function setValue(uint256 value) public {
        slot0 = value;
    }

    function resetValue() public {
        slot0 = 0;
    }

    fallback() external {}
}

contract StorageGasTarget {
    uint256[256] private slots;

    function fill() public {
        for (uint256 i; i < 256; ++i) {
            slots[i] = i + 1;
        }
    }

    function sum() public view returns (uint256 s) {
        for (uint256 i; i < 256; ++i) {
            s += slots[i];
        }
    }
}

contract TargetCreate2 {
    uint256 public value;

    constructor(uint256 value_) {
        value = value_;
    }
}

contract RevertingTarget {
    function fail() public pure {
        revert("failed");
    }
}

contract RevertingConstructor {
    constructor() {
        revert("failed");
    }
}

contract NestedRevertingTarget {
    RevertingTarget public target;

    constructor(RevertingTarget target_) {
        target = target_;
    }

    function fail() public view {
        target.fail();
    }
}

abstract contract LastCallGasFixture is Test {
    Target public target;

    struct Gas {
        uint64 gasTotalUsed;
        uint64 gasMemoryUsed;
        int64 gasRefunded;
    }

    function testRevertNoCachedLastCallGas() public {
        vm._expectCheatcodeRevert();
        vm.lastCallGas();
    }

    function testRevertNoCachedLastFrameGas() public {
        vm._expectCheatcodeRevert();
        vm.lastFrameGas();
    }

    function testLastCallGasDoesNotRecordCreate() public {
        new Target();

        vm._expectCheatcodeRevert();
        vm.lastCallGas();
    }

    function testSnapshotGasLastCallDoesNotRecordCreate() public {
        new Target();

        vm._expectCheatcodeRevert();
        vm.snapshotGasLastCall("testSnapshotGasLastCallDoesNotRecordCreate");
    }

    function _setup() internal {
        // Cannot be set in `setUp` due to `testRevertNoCachedLastCallGas`
        // relying on no calls being made before `lastCallGas` is called.
        target = new Target();
    }

    function _performCall() internal returns (bool success) {
        (success,) = address(target).call("");
    }

    function _performRefund() internal {
        target.setValue(1);
        target.resetValue();
    }

    function _assertGas(Vm.Gas memory lhs, Gas memory rhs) internal {
        assertGt(lhs.gasLimit, 0);
        assertGt(lhs.gasRemaining, 0);
        assertEq(lhs.gasTotalUsed, rhs.gasTotalUsed);
        assertEq(lhs.gasMemoryUsed, rhs.gasMemoryUsed);
        assertEq(lhs.gasRefunded, rhs.gasRefunded);
    }

    function _assertGasRecorded(Vm.Gas memory gas) internal {
        assertGt(gas.gasLimit, 0);
        assertGt(gas.gasRemaining, 0);
        assertGt(gas.gasTotalUsed, 0);
        assertEq(gas.gasMemoryUsed, 0);
    }
}

contract LastFrameGasExpectedRevertTest is Test {
    RevertingTarget public target;

    function setUp() public {
        target = new RevertingTarget();
    }

    function testExpectedRevertCallDoesNotRecordLastFrameGas() public {
        vm.expectRevert();
        target.fail();

        vm._expectCheatcodeRevert();
        vm.lastFrameGas();
    }

    function testExpectedRevertCreateDoesNotRecordLastFrameGas() public {
        vm.expectRevert();
        new RevertingConstructor();

        vm._expectCheatcodeRevert();
        vm.lastFrameGas();
    }

    function testExpectedRevertCreateClearsCachedLastFrameGas() public {
        new Target();

        vm.expectRevert();
        new RevertingConstructor();

        vm._expectCheatcodeRevert();
        vm.lastFrameGas();
    }

    function testNestedExpectedRevertCallClearsCachedLastFrameGas() public {
        NestedRevertingTarget nestedTarget = new NestedRevertingTarget(target);

        vm.expectRevert();
        nestedTarget.fail();

        vm._expectCheatcodeRevert();
        vm.lastFrameGas();
    }

    function testSnapshotGasLastFrameExpectedRevertClearsCachedLastFrameGas() public {
        new Target();

        vm.expectRevert();
        new RevertingConstructor();

        vm._expectCheatcodeRevert();
        vm.snapshotGasLastFrame("testSnapshotGasLastFrameExpectedRevertClearsCachedLastFrameGas");
    }
}

/// forge-config: default.isolate = true
contract LastCallGasIsolatedTest is LastCallGasFixture {
    function testRecordLastFrameGasFromCall() public {
        _setup();
        _performCall();
        _assertGas(vm.lastFrameGas(), Gas({gasTotalUsed: 21064, gasMemoryUsed: 0, gasRefunded: 0}));
    }

    function testRecordLastFrameGasFromCreate() public {
        target = new Target();
        _assertGasRecorded(vm.lastFrameGas());
    }

    function testRecordLastFrameGasFromCreate2() public {
        new TargetCreate2{salt: "salt"}(1);
        _assertGasRecorded(vm.lastFrameGas());
    }

    function testRecordLastCallGas() public {
        _setup();
        _performCall();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 21064, gasMemoryUsed: 0, gasRefunded: 0}));

        _performCall();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 21064, gasMemoryUsed: 0, gasRefunded: 0}));

        _performCall();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 21064, gasMemoryUsed: 0, gasRefunded: 0}));
    }

    function testRecordGasRefund() public {
        _setup();
        _performRefund();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 21380, gasMemoryUsed: 0, gasRefunded: 4800}));
    }

    function testStateDiffRecordingDoesNotWarmStorageReads() public {
        StorageGasTarget recordingOff = new StorageGasTarget();
        recordingOff.fill();
        recordingOff.sum();
        uint64 gasRecordingOff = vm.lastCallGas().gasTotalUsed;

        StorageGasTarget recordingOn = new StorageGasTarget();
        recordingOn.fill();
        vm.startStateDiffRecording();
        recordingOn.sum();

        assertEq(vm.lastCallGas().gasTotalUsed, gasRecordingOff);
    }
}

// Without isolation mode enabled the gas usage will be incorrect.
contract LastCallGasDefaultTest is LastCallGasFixture {
    function testRecordLastFrameGasFromCall() public {
        _setup();
        _performCall();
        _assertGas(vm.lastFrameGas(), Gas({gasTotalUsed: 64, gasMemoryUsed: 0, gasRefunded: 0}));
    }

    function testRecordLastFrameGasFromCreate() public {
        target = new Target();
        _assertGasRecorded(vm.lastFrameGas());
    }

    function testRecordLastFrameGasFromCreate2() public {
        new TargetCreate2{salt: "salt"}(1);
        _assertGasRecorded(vm.lastFrameGas());
    }

    function testRecordLastCallGas() public {
        _setup();
        _performCall();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 64, gasMemoryUsed: 0, gasRefunded: 0}));

        _performCall();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 64, gasMemoryUsed: 0, gasRefunded: 0}));

        _performCall();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 64, gasMemoryUsed: 0, gasRefunded: 0}));
    }

    function testRecordGasRefund() public {
        _setup();
        _performRefund();
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 216, gasMemoryUsed: 0, gasRefunded: 19900}));
    }
}
