// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "ds-test/test.sol";
import "cheats/Vm.sol";

contract SimpleStorage {
    uint256 public value; // Slot 0
    address public owner; // Slot 1
    uint256[3] public values; // Slots 2, 3, 4

    constructor() {
        owner = msg.sender;
    }

    function setValue(uint256 _value) public {
        value = _value;
    }

    function setOwner(address _owner) public {
        owner = _owner;
    }

    function setValues(uint256 a, uint256 b, uint256 c) public {
        values[0] = a;
        values[1] = b;
        values[2] = c;
    }
}

contract VariousArrays {
    // Different array types to test
    uint256[3] public numbers; // Slots 0, 1, 2
    address[2] public addresses; // Slots 3, 4
    bool[5] public flags; // Slot 5 (packed)
    bytes32[2] public hashes; // Slots 6, 7

    function setNumbers(uint256 a, uint256 b, uint256 c) public {
        numbers[0] = a;
        numbers[1] = b;
        numbers[2] = c;
    }

    function setAddresses(address a, address b) public {
        addresses[0] = a;
        addresses[1] = b;
    }

    function setFlags(bool a, bool b, bool c, bool d, bool e) public {
        flags[0] = a;
        flags[1] = b;
        flags[2] = c;
        flags[3] = d;
        flags[4] = e;
    }

    function setHashes(bytes32 a, bytes32 b) public {
        hashes[0] = a;
        hashes[1] = b;
    }
}

contract TwoDArrayStorage {
    // 2D array: 2 arrays of 3 uint256 elements each
    // Total slots: 6 (slots 0-5)
    // [0][0] at slot 0, [0][1] at slot 1, [0][2] at slot 2
    // [1][0] at slot 3, [1][1] at slot 4, [1][2] at slot 5
    uint256[3][2] public matrix;

    // Another 2D array starting at slot 6
    // 3 arrays of 2 addresses each
    // Total slots: 6 (slots 6-11)
    address[2][3] public addresses2D;

    // Mixed size 2D array starting at slot 12
    // 4 arrays of 2 bytes32 each
    // Total slots: 8 (slots 12-19)
    bytes32[2][4] public data2D;

    function setMatrix(uint256[3] memory row0, uint256[3] memory row1) public {
        matrix[0] = row0;
        matrix[1] = row1;
    }

    function setMatrixElement(uint256 i, uint256 j, uint256 value) public {
        matrix[i][j] = value;
    }

    function setAddresses2D(address[2] memory row0, address[2] memory row1, address[2] memory row2) public {
        addresses2D[0] = row0;
        addresses2D[1] = row1;
        addresses2D[2] = row2;
    }

    function setData2D(uint256 i, uint256 j, bytes32 value) public {
        data2D[i][j] = value;
    }
}

contract StateDiffStorageLayoutTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    SimpleStorage simpleStorage;
    VariousArrays variousArrays;
    TwoDArrayStorage twoDArrayStorage;

    function setUp() public {
        simpleStorage = new SimpleStorage();
        variousArrays = new VariousArrays();
        twoDArrayStorage = new TwoDArrayStorage();
    }

    function testSimpleStorageLayout() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Modify storage slots with known positions
        simpleStorage.setValue(42); // Modifies slot 0 (value)
        simpleStorage.setOwner(address(this)); // Modifies slot 1 (owner)
        simpleStorage.setValues(100, 200, 300); // Modifies slots 2, 3, 4 (values array)

        // Get the state diff as string
        string memory stateDiff = vm.getStateDiff();

        // Get the state diff as JSON and verify it contains the expected structure
        string memory stateDiffJson = vm.getStateDiffJson();

        // The JSON should contain storage layout info for all slots
        // We check the JSON contains expected substrings for the labels and types
        assertContains(stateDiffJson, '"label":"value"', "Should contain 'value' label");
        assertContains(stateDiffJson, '"label":"owner"', "Should contain 'owner' label");
        assertContains(stateDiffJson, '"label":"values[0]"', "Should contain 'values[0]' label");
        assertContains(stateDiffJson, '"label":"values[1]"', "Should contain 'values[1]' label");
        assertContains(stateDiffJson, '"label":"values[2]"', "Should contain 'values[2]' label");

        assertContains(stateDiffJson, '"type":"uint256"', "Should contain uint256 type");
        assertContains(stateDiffJson, '"type":"address"', "Should contain address type");
        assertContains(stateDiffJson, '"type":"uint256[3]"', "Should contain uint256[3] type");

        // Check for decoded values
        assertContains(stateDiffJson, '"decoded":', "Should contain decoded values");

        // Check specific decoded values within the decoded object
        // The value 42 should be decoded in the first slot
        assertContains(stateDiffJson, '"decoded":{"previousValue":"0","newValue":"42"}', "Should decode value 42");

        // Check that array values are decoded properly (they will have separate decoded objects)
        assertContains(stateDiffJson, '"newValue":"100"', "Should decode array value 100");
        assertContains(stateDiffJson, '"newValue":"200"', "Should decode array value 200");
        assertContains(stateDiffJson, '"newValue":"300"', "Should decode array value 300");

        // Stop recording and verify we get the expected account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length >= 3, "Should have at least 3 account accesses for the calls");

        // Verify storage accesses for SimpleStorage
        bool foundValueSlot = false;
        bool foundOwnerSlot = false;
        bool foundValuesSlot0 = false;
        bool foundValuesSlot1 = false;
        bool foundValuesSlot2 = false;

        for (uint256 i = 0; i < accesses.length; i++) {
            if (accesses[i].account == address(simpleStorage)) {
                for (uint256 j = 0; j < accesses[i].storageAccesses.length; j++) {
                    bytes32 slot = accesses[i].storageAccesses[j].slot;
                    if (slot == bytes32(uint256(0))) foundValueSlot = true;
                    if (slot == bytes32(uint256(1))) foundOwnerSlot = true;
                    if (slot == bytes32(uint256(2))) foundValuesSlot0 = true;
                    if (slot == bytes32(uint256(3))) foundValuesSlot1 = true;
                    if (slot == bytes32(uint256(4))) foundValuesSlot2 = true;
                }
            }
        }

        assertTrue(foundValueSlot, "Should have accessed slot 0 (value)");
        assertTrue(foundOwnerSlot, "Should have accessed slot 1 (owner)");
        assertTrue(foundValuesSlot0, "Should have accessed slot 2 (values[0])");
        assertTrue(foundValuesSlot1, "Should have accessed slot 3 (values[1])");
        assertTrue(foundValuesSlot2, "Should have accessed slot 4 (values[2])");
    }

    function testVariousArrayTypes() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Modify different array types
        variousArrays.setNumbers(100, 200, 300);
        variousArrays.setAddresses(address(0x1), address(0x2));
        variousArrays.setFlags(true, false, true, false, true);
        variousArrays.setHashes(keccak256("test1"), keccak256("test2"));

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();

        // Verify all array types are properly labeled with indices
        assertContains(stateDiffJson, '"label":"numbers[0]"', "Should contain 'numbers[0]' label");
        assertContains(stateDiffJson, '"label":"numbers[1]"', "Should contain 'numbers[1]' label");
        assertContains(stateDiffJson, '"label":"numbers[2]"', "Should contain 'numbers[2]' label");

        assertContains(stateDiffJson, '"label":"addresses[0]"', "Should contain 'addresses[0]' label");
        assertContains(stateDiffJson, '"label":"addresses[1]"', "Should contain 'addresses[1]' label");

        assertContains(stateDiffJson, '"label":"flags[0]"', "Should contain 'flags[0]' label");

        assertContains(stateDiffJson, '"label":"hashes[0]"', "Should contain 'hashes[0]' label");
        assertContains(stateDiffJson, '"label":"hashes[1]"', "Should contain 'hashes[1]' label");

        // Verify types are correctly identified
        assertContains(stateDiffJson, '"type":"uint256[3]"', "Should contain uint256[3] type");
        assertContains(stateDiffJson, '"type":"address[2]"', "Should contain address[2] type");
        assertContains(stateDiffJson, '"type":"bool[5]"', "Should contain bool[5] type");
        assertContains(stateDiffJson, '"type":"bytes32[2]"', "Should contain bytes32[2] type");

        // Check decoded values
        assertContains(stateDiffJson, '"decoded":', "Should contain decoded values");
        // Check addresses are decoded as raw hex strings
        assertContains(
            stateDiffJson, '"newValue":"0x0000000000000000000000000000000000000001"', "Should decode address 1"
        );
        assertContains(
            stateDiffJson, '"newValue":"0x0000000000000000000000000000000000000002"', "Should decode address 2"
        );

        // Stop recording and verify account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0, "Should have account accesses");
    }

    function testStateDiffJsonFormat() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Make a simple change to verify JSON format
        simpleStorage.setValue(123);

        // Get the JSON and verify it's properly formatted
        string memory stateDiffJson = vm.getStateDiffJson();

        // Check JSON structure contains expected fields
        assertContains(stateDiffJson, '"previousValue":', "JSON should contain previousValue field");
        assertContains(stateDiffJson, '"newValue":', "JSON should contain newValue field");
        assertContains(stateDiffJson, '"label":', "JSON should contain label field");
        assertContains(stateDiffJson, '"type":', "JSON should contain type field");
        assertContains(stateDiffJson, '"offset":', "JSON should contain offset field");
        assertContains(stateDiffJson, '"slot":', "JSON should contain slot field");

        vm.stopAndReturnStateDiff();
    }

    function test2DArrayStorageLayout() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set matrix values
        // matrix[0][0] = 100, matrix[0][1] = 101, matrix[0][2] = 102
        // matrix[1][0] = 200, matrix[1][1] = 201, matrix[1][2] = 202
        uint256[3] memory row0 = [uint256(100), 101, 102];
        uint256[3] memory row1 = [uint256(200), 201, 202];
        twoDArrayStorage.setMatrix(row0, row1);

        // Get the state diff and check labels
        string memory stateDiffJson = vm.getStateDiffJson();

        // Verify the labels for 2D array elements
        assertContains(stateDiffJson, '"label":"matrix[0][0]"', "Should contain matrix[0][0] label");
        assertContains(stateDiffJson, '"label":"matrix[0][1]"', "Should contain matrix[0][1] label");
        assertContains(stateDiffJson, '"label":"matrix[0][2]"', "Should contain matrix[0][2] label");
        assertContains(stateDiffJson, '"label":"matrix[1][0]"', "Should contain matrix[1][0] label");
        assertContains(stateDiffJson, '"label":"matrix[1][1]"', "Should contain matrix[1][1] label");
        assertContains(stateDiffJson, '"label":"matrix[1][2]"', "Should contain matrix[1][2] label");

        // Check that we have the right type
        assertContains(stateDiffJson, '"type":"uint256[3][2]"', "Should contain 2D array type");

        // Check decoded values for 2D arrays
        assertContains(stateDiffJson, '"decoded":', "Should contain decoded values");
        assertContains(stateDiffJson, '"newValue":"100"', "Should decode matrix[0][0] = 100");
        assertContains(stateDiffJson, '"newValue":"101"', "Should decode matrix[0][1] = 101");
        assertContains(stateDiffJson, '"newValue":"102"', "Should decode matrix[0][2] = 102");
        assertContains(stateDiffJson, '"newValue":"200"', "Should decode matrix[1][0] = 200");
        assertContains(stateDiffJson, '"newValue":"201"', "Should decode matrix[1][1] = 201");
        assertContains(stateDiffJson, '"newValue":"202"', "Should decode matrix[1][2] = 202");

        vm.stopAndReturnStateDiff();
    }

    function testMixed2DArrays() public {
        vm.startStateDiffRecording();

        // Test address 2D array
        address[2] memory addrRow0 = [address(0x1), address(0x2)];
        address[2] memory addrRow1 = [address(0x3), address(0x4)];
        address[2] memory addrRow2 = [address(0x5), address(0x6)];
        twoDArrayStorage.setAddresses2D(addrRow0, addrRow1, addrRow2);

        // Test bytes32 2D array
        twoDArrayStorage.setData2D(0, 0, keccak256("data00"));
        twoDArrayStorage.setData2D(0, 1, keccak256("data01"));
        twoDArrayStorage.setData2D(1, 0, keccak256("data10"));
        twoDArrayStorage.setData2D(1, 1, keccak256("data11"));

        string memory stateDiffJson = vm.getStateDiffJson();

        // Check for proper types
        assertContains(stateDiffJson, '"type":"address[2][3]"', "Should contain address 2D array type");
        assertContains(stateDiffJson, '"type":"bytes32[2][4]"', "Should contain bytes32 2D array type");

        // Verify address 2D array labels
        assertContains(stateDiffJson, '"label":"addresses2D[0][0]"', "Should contain addresses2D[0][0] label");
        assertContains(stateDiffJson, '"label":"addresses2D[0][1]"', "Should contain addresses2D[0][1] label");
        assertContains(stateDiffJson, '"label":"addresses2D[1][0]"', "Should contain addresses2D[1][0] label");
        assertContains(stateDiffJson, '"label":"addresses2D[2][1]"', "Should contain addresses2D[2][1] label");

        // Verify data 2D array labels
        assertContains(stateDiffJson, '"label":"data2D[0][0]"', "Should contain data2D[0][0] label");
        assertContains(stateDiffJson, '"label":"data2D[0][1]"', "Should contain data2D[0][1] label");
        assertContains(stateDiffJson, '"label":"data2D[1][0]"', "Should contain data2D[1][0] label");
        assertContains(stateDiffJson, '"label":"data2D[1][1]"', "Should contain data2D[1][1] label");

        vm.stopAndReturnStateDiff();
    }

    function testStateDiffDecodedValues() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Make changes to create state diffs with decoded values
        simpleStorage.setValue(42); // uint256 value
        simpleStorage.setOwner(address(0xBEEF)); // address value
        simpleStorage.setValues(100, 200, 300); // array values

        // Get the state diff as string (not JSON)
        string memory stateDiff = vm.getStateDiff();

        // Test that decoded values are shown in the string format
        // The output uses Unicode arrow →
        // For uint256 values, should show decoded value "42"
        assertContains(stateDiff, unicode"0 → 42", "Should show decoded uint256 value");

        // For addresses, should show decoded address format
        assertContains(stateDiff, "0x000000000000000000000000000000000000bEEF", "Should show decoded address");

        // For array elements, should show decoded values
        assertContains(stateDiff, unicode"0 → 100", "Should show decoded array value 100");
        assertContains(stateDiff, unicode"0 → 200", "Should show decoded array value 200");
        assertContains(stateDiff, unicode"0 → 300", "Should show decoded array value 300");

        vm.stopAndReturnStateDiff();
    }

    // Helper function to check if a string contains a substring
    function assertContains(string memory haystack, string memory needle, string memory message) internal pure {
        bytes memory haystackBytes = bytes(haystack);
        bytes memory needleBytes = bytes(needle);

        if (needleBytes.length > haystackBytes.length) {
            revert(message);
        }

        bool found = false;
        for (uint256 i = 0; i <= haystackBytes.length - needleBytes.length; i++) {
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
