// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "ds-test/test.sol";
import {Vm} from "./Vm.sol";

contract StorageAccessor {
    function read(bytes32 slot) public view returns (bytes32 value) {
        assembly {
            value := sload(slot)
        }
    }

    function write(bytes32 slot, bytes32 value) public {
        assembly {
            sstore(slot, value)
        }
    }
}

contract RecordStorageAccessesTest is DSTest {
    Vm constant cheats = Vm(HEVM_ADDRESS);
    StorageAccessor test1;
    StorageAccessor test2;

    function setUp() public {
        test1 = new StorageAccessor();
        test2 = new StorageAccessor();
    }

    function testRecordAccesses() public {
        StorageAccessor one = test1;
        StorageAccessor two = test2;
        cheats.recordStorageAccesses();
        one.read(bytes32(uint256(1234)));
        one.write(bytes32(uint256(1235)), bytes32(uint256(5678)));
        two.write(bytes32(uint256(5678)), bytes32(uint256(123469)));
        two.write(bytes32(uint256(5678)), bytes32(uint256(1234)));

        Vm.StorageAccess[] memory accessed = cheats.getRecordedStorageAccesses();
        assertEq(accessed.length, 4, "incorrect length");
        Vm.StorageAccess memory access = accessed[0];
        assertEq(access.account, address(one), "incorrect account");
        assertEq(access.slot, bytes32(uint256(1234)), "incorrect slot");
        assertEq(access.isWrite, false);
        assertEq(access.previousValue, bytes32(uint256(0)), "incorrect previousValue");
        assertEq(access.newValue, bytes32(uint256(0)), "incorrect newValue");
        access = accessed[1];
        assertEq(access.account, address(one), "incorrect account");
        assertEq(access.slot, bytes32(uint256(1235)), "incorrect slot");
        assertEq(access.isWrite, true);
        assertEq(access.previousValue, bytes32(uint256(0)), "incorrect previousValue");
        assertEq(access.newValue, bytes32(uint256(5678)), "incorrect newValue");
        access = accessed[2];
        assertEq(access.account, address(two), "incorrect account");
        assertEq(access.slot, bytes32(uint256(5678)), "incorrect slot");
        assertEq(access.isWrite, true);
        assertEq(access.previousValue, bytes32(uint256(0)), "incorrect previousValue");
        assertEq(access.newValue, bytes32(uint256(123469)), "incorrect newValue");
        access = accessed[3];
        assertEq(access.account, address(two), "incorrect account");
        assertEq(access.slot, bytes32(uint256(5678)), "incorrect slot");
        assertEq(access.isWrite, true);
        assertEq(access.previousValue, bytes32(uint256(123469)), "incorrect previousValue");
        assertEq(access.newValue, bytes32(uint256(1234)), "incorrect newValue");
    }
}
