// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Vm.sol";

contract RecordAccess {
    function record() public returns (NestedRecordAccess) {
        assembly {
            sstore(1, add(sload(1), 1))
        }

        NestedRecordAccess inner = new NestedRecordAccess();
        inner.record();

        return inner;
    }
}

contract NestedRecordAccess {
    function record() public {
        assembly {
            sstore(2, add(sload(2), 1))
        }
    }
}

contract RecordTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);

    function testRecordAccess() public {
        RecordAccess target = new RecordAccess();

        // Start recording
        vm.record();
        NestedRecordAccess inner = target.record();

        // Verify Records
        Vm.RecordedAccesses memory access = vm.accesses(address(target));
        Vm.RecordedAccesses memory innerAccess = vm.accesses(address(inner));

        assertEq(access.reads.length, 2, "number of reads is incorrect");
        assertEq(access.reads[0], bytes32(uint256(1)), "key for read 0 is incorrect");
        assertEq(access.reads[1], bytes32(uint256(1)), "key for read 1 is incorrect");

        assertEq(access.writes.length, 1, "number of writes is incorrect");
        assertEq(access.writes[0], bytes32(uint256(1)), "key for write is incorrect");

        assertEq(innerAccess.reads.length, 2, "number of nested reads is incorrect");
        assertEq(innerAccess.reads[0], bytes32(uint256(2)), "key for nested read 0 is incorrect");
        assertEq(innerAccess.reads[1], bytes32(uint256(2)), "key for nested read 1 is incorrect");

        assertEq(innerAccess.writes.length, 1, "number of nested writes is incorrect");
        assertEq(innerAccess.writes[0], bytes32(uint256(2)), "key for nested write is incorrect");
    }
}
