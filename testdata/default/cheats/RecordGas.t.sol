// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract RecordGasTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRecordGasA() public {
        address(0).call("");
        uint64 gasUsed = vm.lastGasUsed();
        emit log_named_uint("gas A", gasUsed);

        address(0).call("");
        gasUsed = vm.lastGasUsed();
        emit log_named_uint("gas B", gasUsed);

        address(0).call("");
        gasUsed = vm.lastGasUsed();
        emit log_named_uint("gas C", gasUsed);
    }
}
