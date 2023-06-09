// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract RecordMapping {
    int256 length;
    mapping(address => int256) data;
    mapping(int256 => mapping(int256 => int256)) nestedData;

    function setData(address addr, int256 value) public {
        data[addr] = value;
    }

    function setNestedData(int256 i, int256 j) public {
        nestedData[i][j] = i * j;
    }
}

contract RecordMappingTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testRecordMapping() public {
        RecordMapping target = new RecordMapping();

        // Start recording
        cheats.startMappingRecording();

        // Verify Records
        target.setData(address(this), 100);
        target.setNestedData(99, 10);
        target.setNestedData(98, 10);
        bool found;
        bytes32 key;
        bytes32 parent;

        bytes32 dataSlot = bytes32(uint256(1));
        bytes32 nestDataSlot = bytes32(uint256(2));
        assertEq(uint256(cheats.getMappingLength(address(target), dataSlot)), 1, "number of data is incorrect");
        assertEq(uint256(cheats.getMappingLength(address(this), dataSlot)), 0, "number of data is incorrect");
        assertEq(
            uint256(cheats.getMappingLength(address(target), nestDataSlot)), 2, "number of nestedData is incorrect"
        );

        bytes32 dataValueSlot = cheats.getMappingSlotAt(address(target), dataSlot, 0);
        (found, key, parent) = cheats.getMappingKeyAndParentOf(address(target), dataValueSlot);
        assert(found);
        assertEq(address(uint160(uint256(key))), address(this), "key of data[i] is incorrect");
        assertEq(parent, dataSlot, "parent of data[i] is incorrect");
        assertGt(uint256(dataValueSlot), 0);
        assertEq(uint256(cheats.load(address(target), dataValueSlot)), 100);

        for (uint256 k; k < cheats.getMappingLength(address(target), nestDataSlot); k++) {
            bytes32 subSlot = cheats.getMappingSlotAt(address(target), nestDataSlot, k);
            (found, key, parent) = cheats.getMappingKeyAndParentOf(address(target), subSlot);
            uint256 i = uint256(key);
            assertEq(parent, nestDataSlot, "parent of nestedData[i][j] is incorrect");
            assertEq(
                uint256(cheats.getMappingLength(address(target), subSlot)), 1, "number of nestedData[i] is incorrect"
            );
            bytes32 leafSlot = cheats.getMappingSlotAt(address(target), subSlot, 0);
            (found, key, parent) = cheats.getMappingKeyAndParentOf(address(target), leafSlot);
            uint256 j = uint256(key);
            assertEq(parent, subSlot, "parent of nestedData[i][j] is incorrect");
            assertEq(j, 10);
            assertEq(uint256(cheats.load(address(target), leafSlot)), i * j, "value of nestedData[i][j] is incorrect");
        }
        cheats.stopMappingRecording();
        assertEq(uint256(cheats.getMappingLength(address(target), dataSlot)), 0, "number of data is incorrect");
    }
}
