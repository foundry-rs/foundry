// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

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

contract StateDiffStorageLayoutTest is Test {
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
        assertTrue(vm.contains(stateDiffJson, '"label":"value"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"owner"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"values[0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"values[1]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"values[2]"'));

        assertTrue(vm.contains(stateDiffJson, '"type":"uint256"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"address"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"uint256[3]"'));

        // Check for decoded values
        assertTrue(vm.contains(stateDiffJson, '"decoded":'));

        // Check specific decoded values within the decoded object
        // The value 42 should be decoded in the first slot
        assertTrue(vm.contains(stateDiffJson, '"decoded":{"previousValue":"0","newValue":"42"}'));

        // Check that array values are decoded properly (they will have separate decoded objects)
        assertTrue(vm.contains(stateDiffJson, '"newValue":"100"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"200"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"300"'));

        // Stop recording and verify we get the expected account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length >= 3);

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

        assertTrue(foundValueSlot);
        assertTrue(foundOwnerSlot);
        assertTrue(foundValuesSlot0);
        assertTrue(foundValuesSlot1);
        assertTrue(foundValuesSlot2);
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
        assertTrue(vm.contains(stateDiffJson, '"label":"numbers[0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"numbers[1]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"numbers[2]"'));

        assertTrue(vm.contains(stateDiffJson, '"label":"addresses[0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"addresses[1]"'));

        assertTrue(vm.contains(stateDiffJson, '"label":"flags[0]"'));

        assertTrue(vm.contains(stateDiffJson, '"label":"hashes[0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"hashes[1]"'));

        // Verify types are correctly identified
        assertTrue(vm.contains(stateDiffJson, '"type":"uint256[3]"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"address[2]"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bool[5]"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes32[2]"'));

        // Check decoded values
        assertTrue(vm.contains(stateDiffJson, '"decoded":'));
        // Check addresses are decoded as raw hex strings
        assertTrue(vm.contains(stateDiffJson, '"newValue":"0x0000000000000000000000000000000000000001"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"0x0000000000000000000000000000000000000002"'));

        // Stop recording and verify account accesses
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0);
    }

    function testStateDiffJsonFormat() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Make a simple change to verify JSON format
        simpleStorage.setValue(123);

        // Get the JSON and verify it's properly formatted
        string memory stateDiffJson = vm.getStateDiffJson();

        // Check JSON structure contains expected fields
        assertTrue(vm.contains(stateDiffJson, '"previousValue":'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":'));
        assertTrue(vm.contains(stateDiffJson, '"label":'));
        assertTrue(vm.contains(stateDiffJson, '"type":'));
        assertTrue(vm.contains(stateDiffJson, '"offset":'));
        assertTrue(vm.contains(stateDiffJson, '"slot":'));

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
        assertTrue(vm.contains(stateDiffJson, '"label":"matrix[0][0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"matrix[0][1]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"matrix[0][2]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"matrix[1][0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"matrix[1][1]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"matrix[1][2]"'));

        // Check that we have the right type
        assertTrue(vm.contains(stateDiffJson, '"type":"uint256[3][2]"'));

        // Check decoded values for 2D arrays
        assertTrue(vm.contains(stateDiffJson, '"decoded":'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"100"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"101"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"102"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"200"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"201"'));
        assertTrue(vm.contains(stateDiffJson, '"newValue":"202"'));

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
        assertTrue(vm.contains(stateDiffJson, '"type":"address[2][3]"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes32[2][4]"'));

        // Verify address 2D array labels
        assertTrue(vm.contains(stateDiffJson, '"label":"addresses2D[0][0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"addresses2D[0][1]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"addresses2D[1][0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"addresses2D[2][1]"'));

        // Verify data 2D array labels
        assertTrue(vm.contains(stateDiffJson, '"label":"data2D[0][0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"data2D[0][1]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"data2D[1][0]"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"data2D[1][1]"'));

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
        assertTrue(vm.contains(stateDiff, unicode"0 → 42"));

        // For addresses, should show decoded address format
        assertTrue(vm.contains(stateDiff, "0x000000000000000000000000000000000000bEEF"));

        // For array elements, should show decoded values
        assertTrue(vm.contains(stateDiff, unicode"0 → 100"));
        assertTrue(vm.contains(stateDiff, unicode"0 → 200"));
        assertTrue(vm.contains(stateDiff, unicode"0 → 300"));

        vm.stopAndReturnStateDiff();
    }
}
