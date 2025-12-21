// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "utils/Test.sol";

contract DiffTest {
    // slot 0
    struct TestStruct {
        uint128 a;
        uint128 b;
    }

    // Multi-slot struct (spans 3 slots)
    struct MultiSlotStruct {
        uint256 value1; // slot 1
        address addr; // slot 2 (takes 20 bytes, but uses full slot)
        uint256 value2; // slot 3
    }

    // Nested struct with MultiSlotStruct as inner
    struct NestedStruct {
        MultiSlotStruct inner; // slots 4-6 (spans 3 slots)
        uint256 value; // slot 7
        address owner; // slot 8
    }

    TestStruct internal testStruct;
    MultiSlotStruct internal multiSlotStruct;
    NestedStruct internal nestedStruct;

    function setStruct(uint128 a, uint128 b) public {
        testStruct.a = a;
        testStruct.b = b;
    }

    function setMultiSlotStruct(uint256 v1, address a, uint256 v2) public {
        multiSlotStruct.value1 = v1;
        multiSlotStruct.addr = a;
        multiSlotStruct.value2 = v2;
    }

    function setNestedStruct(uint256 v1, address a, uint256 v2, uint256 v, address o) public {
        nestedStruct.inner.value1 = v1;
        nestedStruct.inner.addr = a;
        nestedStruct.inner.value2 = v2;
        nestedStruct.value = v;
        nestedStruct.owner = o;
    }
}

contract StateDiffStructTest is Test {
    DiffTest internal test;

    function setUp() public {
        test = new DiffTest();
    }

    function testStruct() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set struct values: a=1, b=2
        test.setStruct(1, 2);

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();

        // Debug: log the JSON for inspection
        emit log_string("State diff JSON (testdata):");
        emit log_string(stateDiffJson);

        // Check that the struct is properly labeled
        assertTrue(vm.contains(stateDiffJson, '"label":"testStruct"'));

        // Check that the type is correctly identified as a struct
        assertTrue(vm.contains(stateDiffJson, '"type":"struct DiffTest.TestStruct"'));

        // Check for members field - structs have members with individual decoded values
        assertTrue(vm.contains(stateDiffJson, '"members":'));

        // Check that member 'a' is properly decoded
        assertTrue(vm.contains(stateDiffJson, '"label":"a"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"uint128"'));

        // Check that member 'b' is properly decoded
        assertTrue(vm.contains(stateDiffJson, '"label":"b"'));

        // The members should have decoded values
        // Check specific decoded values for each member in the members array
        // Member 'a' at offset 0 should have previous value 0 and new value 1
        assertTrue(
            vm.contains(
                stateDiffJson,
                '{"label":"a","type":"uint128","offset":0,"slot":"0","decoded":{"previousValue":"0","newValue":"1"}}'
            )
        );

        // Member 'b' at offset 16 should have previous value 0 and new value 2
        assertTrue(
            vm.contains(
                stateDiffJson,
                '{"label":"b","type":"uint128","offset":16,"slot":"0","decoded":{"previousValue":"0","newValue":"2"}}'
            )
        );

        // Verify the raw storage values are correct
        // The storage layout packs uint128 a at offset 0 and uint128 b at offset 16
        // So the value 0x0000000000000000000200000000000000000000000000000001 represents:
        // - First 16 bytes (a): 0x0000000000000000000000000000000001 = 1
        // - Last 16 bytes (b):  0x0000000000000000000000000000000002 = 2
        assertTrue(vm.contains(stateDiffJson, '"0x0000000000000000000000000000000200000000000000000000000000000001"'));

        // Stop recording and verify we get the expected account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0);

        // Find the storage access for our struct
        bool foundStructAccess = false;
        for (uint256 i = 0; i < accesses.length; i++) {
            if (accesses[i].account == address(test)) {
                for (uint256 j = 0; j < accesses[i].storageAccesses.length; j++) {
                    Vm.StorageAccess memory access = accesses[i].storageAccesses[j];
                    if (access.slot == bytes32(uint256(0)) && access.isWrite) {
                        foundStructAccess = true;
                        // Verify the storage values
                        assertEq(access.previousValue, bytes32(uint256(0)));
                        assertEq(
                            access.newValue, bytes32(uint256(0x0000000000000000000200000000000000000000000000000001))
                        );
                    }
                }
            }
        }

        assertTrue(foundStructAccess);
    }

    function testMultiSlotStruct() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set multi-slot struct values
        test.setMultiSlotStruct(
            123456789, // value1
            address(0xdEaDbEeF), // addr
            987654321 // value2
        );

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();

        // Debug: log the JSON for inspection
        emit log_string("State diff JSON:");
        emit log_string(stateDiffJson);

        // Check that the struct's first member is properly labeled
        assertTrue(vm.contains(stateDiffJson, '"label":"multiSlotStruct.value1"'));

        // For multi-slot structs, the base slot now shows the first member's type
        // The struct type itself is not shown since we decode the first member directly

        // Multi-slot structs don't have members field in the base slot
        // Instead, each member appears as a separate slot entry with dotted labels

        // Check that each member slot is properly labeled
        // Note: slot 1 now shows multiSlotStruct.value1 since it's the first member
        assertTrue(vm.contains(stateDiffJson, '"label":"multiSlotStruct.value1"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"multiSlotStruct.addr"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"multiSlotStruct.value2"'));

        // Check member types
        assertTrue(vm.contains(stateDiffJson, '"type":"uint256"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"address"'));

        // Check that value1 is properly decoded from slot 1
        assertTrue(vm.contains(stateDiffJson, '"decoded":{"previousValue":"0","newValue":"123456789"}'));

        // Also verify the raw hex value
        assertTrue(vm.contains(stateDiffJson, "0x00000000000000000000000000000000000000000000000000000000075bcd15"));

        // Slot 2 should have the address decoded
        assertTrue(
            vm.contains(
                stateDiffJson,
                '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x00000000000000000000000000000000DeaDBeef"}'
            )
        );

        // Slot 3 should have value2 decoded
        assertTrue(vm.contains(stateDiffJson, '"decoded":{"previousValue":"0","newValue":"987654321"}'));

        // Stop recording
        vm.stopAndReturnStateDiff();
    }

    function testNestedStruct() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set nested struct values - now with MultiSlotStruct as inner
        test.setNestedStruct(
            111111111, // inner.value1
            address(0xCAFE), // inner.addr
            222222222, // inner.value2
            333333333, // value
            address(0xBEEF) // owner
        );

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();

        // Debug: log the JSON for inspection
        emit log_string("State diff JSON (testdata):");
        emit log_string(stateDiffJson);

        assertTrue(
            vm.contains(
                stateDiffJson,
                '"label":"nestedStruct.inner.value1","type":"uint256","offset":0,"slot":"4","decoded":{"previousValue":"0","newValue":"111111111"}'
            )
        );

        assertTrue(
            vm.contains(
                stateDiffJson,
                '"label":"nestedStruct.inner.addr","type":"address","offset":0,"slot":"5","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x000000000000000000000000000000000000cafE"}'
            )
        );

        assertTrue(
            vm.contains(
                stateDiffJson,
                '"label":"nestedStruct.inner.value2","type":"uint256","offset":0,"slot":"6","decoded":{"previousValue":"0","newValue":"222222222"}'
            )
        );

        assertTrue(vm.contains(stateDiffJson, "0x00000000000000000000000000000000000000000000000000000000069f6bc7"));

        assertTrue(
            vm.contains(
                stateDiffJson,
                '"label":"nestedStruct.value","type":"uint256","offset":0,"slot":"7","decoded":{"previousValue":"0","newValue":"333333333"}'
            )
        );

        assertTrue(
            vm.contains(
                stateDiffJson,
                '"label":"nestedStruct.owner","type":"address","offset":0,"slot":"8","decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x000000000000000000000000000000000000bEEF"}'
            )
        );

        // Stop recording
        vm.stopAndReturnStateDiff();
    }
}
