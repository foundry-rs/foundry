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

    function failWithInvalid() public pure {
        assembly {
            invalid()
        }
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

    function writeAll() public {
        for (uint256 i; i < 256; ++i) {
            slots[i] = i + 2;
        }
    }

    function sum() public view returns (uint256 s) {
        for (uint256 i; i < 256; ++i) {
            s += slots[i];
        }
    }
}

contract AccountGasTarget {
    function balanceOf(address account) public view returns (uint256) {
        return account.balance;
    }

    function codeSizeOf(address account) public view returns (uint256 size) {
        assembly {
            size := extcodesize(account)
        }
    }

    function codeHashOf(address account) public view returns (bytes32 hash) {
        assembly {
            hash := extcodehash(account)
        }
    }

    function copyCodeOf(address account) public view returns (bytes32 word) {
        assembly {
            extcodecopy(account, 0, 0, 0x20)
            word := mload(0)
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

contract RefundingConstructor {
    constructor(Target target) {
        target.resetValue();
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
        assertEq(lhs.gasStateUsed, 0);
    }

    function _assertGasRecorded(Vm.Gas memory gas) internal {
        assertGt(gas.gasLimit, 0);
        assertGt(gas.gasRemaining, 0);
        assertGt(gas.gasTotalUsed, 0);
        assertEq(gas.gasMemoryUsed, 0);
        assertEq(gas.gasStateUsed, 0);
    }
}

contract LastCallGasConstructorTest is Test {
    Target public target;

    constructor() {
        target = new Target();
        target.setValue(1);
    }

    function testConstructorCallIsRecorded() public {
        Vm.Gas memory gas = vm.lastCallGas();
        assertGt(gas.gasLimit, 0);
        assertGt(gas.gasRemaining, 0);
        assertGt(gas.gasTotalUsed, 0);
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
        _assertGas(vm.lastCallGas(), Gas({gasTotalUsed: 26180, gasMemoryUsed: 0, gasRefunded: 4800}));
        assertEq(vm.snapshotGasLastCall("isolated refund call"), 21380);
        assertEq(vm.snapshotGasLastFrame("isolated refund frame"), 21380);
    }

    function testSnapshotGasSectionRefund() public {
        _snapshotResetValue(1);
    }

    function testSnapshotGasSectionNoRefund() public {
        _snapshotResetValue(0);
    }

    function testSnapshotGasSectionAfterRefund() public {
        _setup();
        _performRefund();
        _snapshotResetValue(0);
    }

    function testSnapshotGasSectionMultipleRefunds() public {
        _setup();
        Target other = new Target();
        target.setValue(1);
        other.setValue(1);
        vm.startSnapshotGas("isolated multiple refunds");
        target.resetValue();
        other.resetValue();
        // Recorded with v1.8.1, including both finalized transaction refunds.
        assertEq(vm.stopSnapshotGas(), 43648);
    }

    /// forge-config: default.evm_version = "cancun"
    function testSnapshotGasSectionCreateRefund() public {
        _setup();
        target.setValue(1);
        // Prepare the init code before measuring so compiler-dependent copying is excluded.
        bytes memory initCode = abi.encodePacked(type(RefundingConstructor).creationCode, abi.encode(target));
        vm.startSnapshotGas("isolated create refund");
        assembly {
            pop(create(0, add(initCode, 32), mload(initCode)))
        }
        uint256 section = vm.stopSnapshotGas();
        // CREATE costs 32000 gas; the remaining 20 gas is the pre-v1.8.2 snapshot overhead.
        // EIP-3860 additionally charges 2 gas per init code word outside the isolated frame.
        uint256 initCodeCost = 2 * ((initCode.length + 31) / 32);
        assertEq(section, vm.snapshotGasLastFrame("isolated create frame") + 32020 + initCodeCost);
    }

    function _snapshotResetValue(uint256 initialValue) internal {
        _setup();
        target.setValue(initialValue);
        vm.startSnapshotGas("isolated section");
        target.resetValue();
        uint256 section = vm.stopSnapshotGas();
        // Preserve the pre-v1.8.3 region overhead for both refunding and non-refunding calls.
        assertEq(section, vm.snapshotGasLastCall("isolated section call") + 543);
    }

    function testSnapshotGasForFailedCharge() public {
        _setup();
        (bool success,) = address(target).call{gas: 100_000}(abi.encodeCall(target.failWithInvalid, ()));
        assertEq(success, false);
        assertEq(vm.snapshotGasLastCall("isolated failed charge call"), 0);
        assertEq(vm.snapshotGasLastFrame("isolated failed charge frame"), 0);
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

    function testStateDiffRecordingDoesNotWarmStorageWrites() public {
        StorageGasTarget recordingOff = new StorageGasTarget();
        recordingOff.fill();
        recordingOff.writeAll();
        uint64 gasRecordingOff = vm.lastCallGas().gasTotalUsed;

        StorageGasTarget recordingOn = new StorageGasTarget();
        recordingOn.fill();
        vm.startStateDiffRecording();
        recordingOn.writeAll();

        assertEq(vm.lastCallGas().gasTotalUsed, gasRecordingOff);
    }

    function testStateDiffRecordingDoesNotWarmBalanceReads() public {
        AccountGasTarget recordingOff = new AccountGasTarget();
        Target accountOff = new Target();
        recordingOff.balanceOf(address(accountOff));
        uint64 gasRecordingOff = vm.lastCallGas().gasTotalUsed;

        AccountGasTarget recordingOn = new AccountGasTarget();
        Target accountOn = new Target();
        vm.startStateDiffRecording();
        recordingOn.balanceOf(address(accountOn));

        assertEq(vm.lastCallGas().gasTotalUsed, gasRecordingOff);
    }

    function testStateDiffRecordingDoesNotWarmExtcodesizeReads() public {
        AccountGasTarget recordingOff = new AccountGasTarget();
        Target accountOff = new Target();
        recordingOff.codeSizeOf(address(accountOff));
        uint64 gasRecordingOff = vm.lastCallGas().gasTotalUsed;

        AccountGasTarget recordingOn = new AccountGasTarget();
        Target accountOn = new Target();
        vm.startStateDiffRecording();
        recordingOn.codeSizeOf(address(accountOn));

        assertEq(vm.lastCallGas().gasTotalUsed, gasRecordingOff);
    }

    function testStateDiffRecordingDoesNotWarmExtcodehashReads() public {
        AccountGasTarget recordingOff = new AccountGasTarget();
        Target accountOff = new Target();
        recordingOff.codeHashOf(address(accountOff));
        uint64 gasRecordingOff = vm.lastCallGas().gasTotalUsed;

        AccountGasTarget recordingOn = new AccountGasTarget();
        Target accountOn = new Target();
        vm.startStateDiffRecording();
        recordingOn.codeHashOf(address(accountOn));

        assertEq(vm.lastCallGas().gasTotalUsed, gasRecordingOff);
    }

    function testStateDiffRecordingDoesNotWarmExtcodecopyReads() public {
        AccountGasTarget recordingOff = new AccountGasTarget();
        Target accountOff = new Target();
        recordingOff.copyCodeOf(address(accountOff));
        uint64 gasRecordingOff = vm.lastCallGas().gasTotalUsed;

        AccountGasTarget recordingOn = new AccountGasTarget();
        Target accountOn = new Target();
        vm.startStateDiffRecording();
        recordingOn.copyCodeOf(address(accountOn));

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
        assertEq(vm.snapshotGasLastCall("refund call"), 216);
        assertEq(vm.snapshotGasLastFrame("refund frame"), 216);
    }
}
