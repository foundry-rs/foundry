// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RecordGasTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRecordGasA() public {
        address(0).call("");
        Vm.Gas memory record = vm.lastGasUsed();
        _logGasRecord(record);

        address(0).call("");
        record = vm.lastGasUsed();
        _logGasRecord(record);

        address(0).call("");
        record = vm.lastGasUsed();
        _logGasRecord(record);
    }

    function _logGasRecord(Vm.Gas memory record) internal {
        emit log_named_uint("gasLimit", record.gasLimit);
        emit log_named_uint("gasTotalUsed", record.gasTotalUsed);
        emit log_named_uint("gasMemoryUsed", record.gasMemoryUsed);
        emit log_named_int("gasRefunded", record.gasRefunded);
        emit log_named_uint("gasRemaining", record.gasRemaining);
    }
}
