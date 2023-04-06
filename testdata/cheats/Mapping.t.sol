// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract RecordMapping {
    int length;
    mapping(address => int) data;
    mapping(int => mapping(int => int)) nestedData;

    function setData(address addr, int value) {
        data[addr] = value;
    }

    function setNestedData(int i, int j, int value) {
        nestedData[i][j] = value;
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
        target.setNestedData(99, 10, 99*10);
        target.setNestedData(98, 10, 98*10);

        uint dataSlot = 1;
        uint nestDataSlot = 2;
        assertEq(cheats.getMappingLength(dataSlot), 1, "number of data is incorrect");
        assertEq(cheats.getMappingLength(nestDataSlot), 2, "number of nestedData is incorrect");

        uint dataValueSlot = cheats.getMappingSlotAt(dataSlot, 0);
        assertEq(uint(cheats.load(target, dataValueSlot)), 100);

        for (int i; i < cheats.getMappingLength(nestDataSlot); i++) {

        }
    }
}
