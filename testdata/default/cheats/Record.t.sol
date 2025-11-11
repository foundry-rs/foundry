// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

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

contract RecordTest is Test {
    function testRecordAccess() public {
        RecordAccess target = new RecordAccess();

        // Start recording
        vm.record();
        NestedRecordAccess inner = target.record();

        // Verify Records
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(target));
        (bytes32[] memory innerReads, bytes32[] memory innerWrites) = vm.accesses(address(inner));

        assertEq(reads.length, 2, "number of reads is incorrect");
        assertEq(reads[0], bytes32(uint256(1)), "key for read 0 is incorrect");
        assertEq(reads[1], bytes32(uint256(1)), "key for read 1 is incorrect");

        assertEq(writes.length, 1, "number of writes is incorrect");
        assertEq(writes[0], bytes32(uint256(1)), "key for write is incorrect");

        assertEq(innerReads.length, 2, "number of nested reads is incorrect");
        assertEq(innerReads[0], bytes32(uint256(2)), "key for nested read 0 is incorrect");
        assertEq(innerReads[1], bytes32(uint256(2)), "key for nested read 1 is incorrect");

        assertEq(innerWrites.length, 1, "number of nested writes is incorrect");
        assertEq(innerWrites[0], bytes32(uint256(2)), "key for nested write is incorrect");
    }

    function testStopRecordAccess() public {
        RecordAccess target = new RecordAccess();

        // Start recording
        vm.record();
        NestedRecordAccess inner = target.record();

        // Verify Records
        (bytes32[] memory reads, bytes32[] memory writes) = vm.accesses(address(target));

        assertEq(reads.length, 2, "number of reads is incorrect");
        assertEq(reads[0], bytes32(uint256(1)), "key for read 0 is incorrect");
        assertEq(reads[1], bytes32(uint256(1)), "key for read 1 is incorrect");

        assertEq(writes.length, 1, "number of writes is incorrect");
        assertEq(writes[0], bytes32(uint256(1)), "key for write is incorrect");

        vm.stopRecord();
        inner = target.record();

        // Verify that there are no new Records
        (reads, writes) = vm.accesses(address(target));

        assertEq(reads.length, 2, "number of reads is incorrect");
        assertEq(reads[0], bytes32(uint256(1)), "key for read 0 is incorrect");
        assertEq(reads[1], bytes32(uint256(1)), "key for read 1 is incorrect");

        assertEq(writes.length, 1, "number of writes is incorrect");
        assertEq(writes[0], bytes32(uint256(1)), "key for write is incorrect");

        vm.record();
        vm.stopRecord();

        // verify reset all records
        (reads, writes) = vm.accesses(address(target));

        assertEq(reads.length, 0, "number of reads is incorrect");
        assertEq(writes.length, 0, "number of writes is incorrect");
    }
}
