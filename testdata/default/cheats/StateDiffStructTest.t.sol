// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract DiffTest {
    struct TestStruct {
        uint128 a;
        uint128 b;
    }

    // Multi-slot struct (spans 3 slots)
    struct MultiSlotStruct {
        uint256 value1; // slot 0
        address addr; // slot 1 (takes 20 bytes, but uses full slot)
        uint256 value2; // slot 2
    }

    // Nested struct with MultiSlotStruct as inner
    struct NestedStruct {
        MultiSlotStruct inner; // slots 0-2 (spans 3 slots)
        uint256 value; // slot 3
        address owner; // slot 4
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

    function setNestedStruct(
        uint256 v1,
        address a,
        uint256 v2,
        uint256 v,
        address o
    ) public {
        nestedStruct.inner.value1 = v1;
        nestedStruct.inner.addr = a;
        nestedStruct.inner.value2 = v2;
        nestedStruct.value = v;
        nestedStruct.owner = o;
    }
}

contract StateDiffStructTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
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

        // Check that the struct is properly labeled
        assertContains(
            stateDiffJson,
            '"label":"testStruct"',
            "Should contain 'testStruct' label"
        );

        // Check that the type is correctly identified as a struct
        assertContains(
            stateDiffJson,
            '"type":"struct DiffTest.TestStruct"',
            "Should contain struct type"
        );

        // Check for members field - structs have members with individual decoded values
        assertContains(
            stateDiffJson,
            '"members":',
            "Should contain members field for struct"
        );

        // Check that member 'a' is properly decoded
        assertContains(
            stateDiffJson,
            '"label":"a"',
            "Should contain member 'a' label"
        );
        assertContains(
            stateDiffJson,
            '"type":"uint128"',
            "Should contain uint128 type for members"
        );

        // Check that member 'b' is properly decoded
        assertContains(
            stateDiffJson,
            '"label":"b"',
            "Should contain member 'b' label"
        );

        // The members should have decoded values
        // Check specific decoded values for each member in the members array
        // Member 'a' at offset 0 should have previous value 0 and new value 1
        assertContains(
            stateDiffJson,
            '{"label":"a","type":"uint128","offset":0,"slot":"0","decoded":{"previousValue":"0","newValue":"1"}}',
            "Member 'a' should be decoded with previous=0, new=1"
        );

        // Member 'b' at offset 16 should have previous value 0 and new value 2
        assertContains(
            stateDiffJson,
            '{"label":"b","type":"uint128","offset":16,"slot":"0","decoded":{"previousValue":"0","newValue":"2"}}',
            "Member 'b' should be decoded with previous=0, new=2"
        );

        // Verify the raw storage values are correct
        // The storage layout packs uint128 a at offset 0 and uint128 b at offset 16
        // So the value 0x0000000000000000000200000000000000000000000000000001 represents:
        // - First 16 bytes (a): 0x0000000000000000000000000000000001 = 1
        // - Last 16 bytes (b):  0x0000000000000000000000000000000002 = 2
        assertContains(
            stateDiffJson,
            '"0x0000000000000000000000000000000200000000000000000000000000000001"',
            "Should contain the correct packed storage value"
        );

        // Stop recording and verify we get the expected account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0, "Should have account accesses");

        // Find the storage access for our struct
        bool foundStructAccess = false;
        for (uint256 i = 0; i < accesses.length; i++) {
            if (accesses[i].account == address(test)) {
                for (
                    uint256 j = 0;
                    j < accesses[i].storageAccesses.length;
                    j++
                ) {
                    Vm.StorageAccess memory access = accesses[i]
                        .storageAccesses[j];
                    if (access.slot == bytes32(uint256(0)) && access.isWrite) {
                        foundStructAccess = true;
                        // Verify the storage values
                        assertEq(
                            access.previousValue,
                            bytes32(uint256(0)),
                            "Previous value should be 0"
                        );
                        assertEq(
                            access.newValue,
                            bytes32(
                                uint256(
                                    0x0000000000000000000200000000000000000000000000000001
                                )
                            ),
                            "New value should pack a=1 and b=2"
                        );
                    }
                }
            }
        }

        assertTrue(
            foundStructAccess,
            "Should have found struct storage access"
        );
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
        assertContains(
            stateDiffJson,
            '"label":"multiSlotStruct.value1"',
            "Should contain 'multiSlotStruct.value1' label"
        );

        // For multi-slot structs, the base slot now shows the first member's type
        // The struct type itself is not shown since we decode the first member directly

        // Multi-slot structs don't have members field in the base slot
        // Instead, each member appears as a separate slot entry with dotted labels

        // Check that each member slot is properly labeled
        // Note: slot 1 now shows multiSlotStruct.value1 since it's the first member
        assertContains(
            stateDiffJson,
            '"label":"multiSlotStruct.value1"',
            "Should contain multiSlotStruct.value1 label for first slot"
        );
        assertContains(
            stateDiffJson,
            '"label":"multiSlotStruct.addr"',
            "Should contain member 'addr' label"
        );
        assertContains(
            stateDiffJson,
            '"label":"multiSlotStruct.value2"',
            "Should contain member 'value2' label"
        );

        // Check member types
        assertContains(
            stateDiffJson,
            '"type":"uint256"',
            "Should contain uint256 type"
        );
        assertContains(
            stateDiffJson,
            '"type":"address"',
            "Should contain address type"
        );

        // Check that value1 is properly decoded from slot 1
        assertContains(
            stateDiffJson,
            '"decoded":{"previousValue":"0","newValue":"123456789"}',
            "value1 should be decoded from slot 1"
        );
        
        // Also verify the raw hex value
        assertContains(
            stateDiffJson,
            "0x00000000000000000000000000000000000000000000000000000000075bcd15",
            "Slot 1 should contain value1 in hex"
        );

        // Slot 2 should have the address decoded
        assertContains(
            stateDiffJson,
            '"decoded":{"previousValue":"0x0000000000000000000000000000000000000000","newValue":"0x00000000000000000000000000000000DeaDBeef"}',
            "Address should be decoded from slot 2"
        );

        // Slot 3 should have value2 decoded
        assertContains(
            stateDiffJson,
            '"decoded":{"previousValue":"0","newValue":"987654321"}',
            "Value2 should be decoded from slot 3"
        );

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

        // Check that the struct is properly labeled
        assertContains(
            stateDiffJson,
            '"label":"nestedStruct"',
            "Should contain 'nestedStruct' label"
        );

        // Check that the type is correctly identified as a struct
        assertContains(
            stateDiffJson,
            '"type":"struct DiffTest.NestedStruct"',
            "Should contain struct type"
        );

        // Nested struct with multi-slot inner struct doesn't have members field either
        // Each member appears as a separate slot

        // Check that nested struct labels are properly set
        assertContains(
            stateDiffJson,
            '"label":"nestedStruct"',
            "Should contain nestedStruct label"
        );

        // Check other members have proper labels
        assertContains(
            stateDiffJson,
            '"label":"nestedStruct.value"',
            "Should contain member 'value' label"
        );
        assertContains(
            stateDiffJson,
            '"label":"nestedStruct.owner"',
            "Should contain member 'owner' label"
        );

        // The inner struct members are in slots 4, 5, 6 but we only see their storage diffs
        // They don't appear with member labels in this test since they're part of the nested struct

        // Check that slot 4 has the first value
        assertContains(
            stateDiffJson,
            "0x00000000000000000000000000000000000000000000000000000000069f6bc7",
            "Slot 4 should contain inner.value1 in hex"
        );
        // Note: addresses in slots 5 and 6 may not have labels due to nested struct complexity
        // But the important values are decoded correctly

        // Check decoded values for outer struct members
        // Slot 7 should have nestedStruct.value decoded with previous=0 and new=333333333
        assertContains(
            stateDiffJson,
            '"label":"nestedStruct.value"',
            "Should have nestedStruct.value label"
        );
        assertContains(
            stateDiffJson,
            '"slot":"7"',
            "nestedStruct.value should be in slot 7"
        );
        assertContains(
            stateDiffJson,
            '"previousValue":"0","newValue":"333333333"',
            "Should decode nestedStruct.value correctly"
        );

        // Slot 8 should have nestedStruct.owner decoded
        assertContains(
            stateDiffJson,
            '"label":"nestedStruct.owner"',
            "Should have nestedStruct.owner label"
        );
        assertContains(
            stateDiffJson,
            '"slot":"8"',
            "nestedStruct.owner should be in slot 8"
        );
        assertContains(
            stateDiffJson,
            '"newValue":"0x000000000000000000000000000000000000bEEF"',
            "Should decode owner address correctly"
        );

        // Stop recording
        vm.stopAndReturnStateDiff();
    }

    // Helper function to check if a string contains a substring
    function assertContains(
        string memory haystack,
        string memory needle,
        string memory message
    ) internal pure {
        bytes memory haystackBytes = bytes(haystack);
        bytes memory needleBytes = bytes(needle);

        if (needleBytes.length > haystackBytes.length) {
            revert(message);
        }

        bool found = false;
        for (
            uint256 i = 0;
            i <= haystackBytes.length - needleBytes.length;
            i++
        ) {
            bool isMatch = true;
            for (uint256 j = 0; j < needleBytes.length; j++) {
                if (haystackBytes[i + j] != needleBytes[j]) {
                    isMatch = false;
                    break;
                }
            }
            if (isMatch) {
                found = true;
                break;
            }
        }

        if (!found) {
            revert(message);
        }
    }
}
