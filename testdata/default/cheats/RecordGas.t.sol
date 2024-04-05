// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract Target {
    uint256 public a;

    function expandMemory() public pure returns (uint256) {
        uint256[] memory arr = new uint256[](1000);

        for (uint256 i = 0; i < arr.length; i++) {
            arr[i] = i;
        }

        return arr.length;
    }

    function set(uint256 _a) public {
        a = _a;
    }
}

contract RecordGasTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    Target public target;

    function testNoGasRecord() public {
        Vm.Gas memory record = vm.lastGasUsed();
        assertEq(record.gasLimit, 0);
        assertEq(record.gasTotalUsed, 0);
        assertEq(record.gasMemoryUsed, 0);
        assertEq(record.gasRefunded, 0);
        assertEq(record.gasRemaining, 0);
    }

    function testRecordGas() public {
        address(0).call("");
        _logGasRecord();

        address(0).call("");
        _logGasRecord();

        address(0).call("");
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
        target.set(0);
        _logGasRecord();
    }

    function testRecordGasSingleField() public {
        address(0).call("");
        _logGasTotalUsed();
    }

    function _logGasTotalUsed() internal {
        uint256 gasTotalUsed = vm.lastGasUsed().gasTotalUsed;
        emit log_named_uint("gasTotalUsed", gasTotalUsed);
    }

    function _logGasRecord() internal {
        Vm.Gas memory record = vm.lastGasUsed();
        emit log_named_uint("gasLimit", record.gasLimit);
        emit log_named_uint("gasTotalUsed", record.gasTotalUsed);
        emit log_named_uint("gasMemoryUsed", record.gasMemoryUsed);
        emit log_named_int("gasRefunded", record.gasRefunded);
        emit log_named_uint("gasRemaining", record.gasRemaining);
        emit log_string("");
    }
}
