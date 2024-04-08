// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Target {
    uint256 public slot0;

    function expandMemory() public pure returns (uint256) {
        uint256[] memory arr = new uint256[](1000);

        for (uint256 i = 0; i < arr.length; i++) {
            arr[i] = i;
        }

        return arr.length;
    }

    function set(uint256 value) public {
        slot0 = value;
    }

    function reset() public {
        slot0 = 0;
    }
}

contract RecordGasTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Target public target;

    function testNoGasRecord() public {
        vm.expectRevert();
        Vm.Gas memory record = vm.lastCallGas();
    }

    function testRecordGas() public {
        target = new Target();

        _performCall();
        _logGasRecord();

        _performCall();
        _logGasRecord();

        _performCall();
        _logGasRecord();
    }

    function testRecordGasMemory() public {
        target = new Target();
        target.expandMemory();
        _logGasRecord();
    }

    function testRecordGasRefund() public {
        target = new Target();
        target.set(1);
        target.reset();
        _logGasRecord();
    }

    function testRecordGasSingleField() public {
        _performCall();
        _logGasTotalUsed();
    }

    function _performCall() internal returns (bool success) {
        (success, ) = address(0).call("");
    }

    function _logGasTotalUsed() internal {
        uint256 gasTotalUsed = vm.lastCallGas().gasTotalUsed;
        emit log_named_uint("gasTotalUsed", gasTotalUsed);
    }

    function _logGasRecord() internal {
        Vm.Gas memory record = vm.lastCallGas();
        emit log_named_uint("gasLimit", record.gasLimit);
        emit log_named_uint("gasTotalUsed", record.gasTotalUsed);
        emit log_named_uint("gasMemoryUsed", record.gasMemoryUsed);
        emit log_named_int("gasRefunded", record.gasRefunded);
        emit log_named_uint("gasRemaining", record.gasRemaining);
        emit log_string("");
    }
}
