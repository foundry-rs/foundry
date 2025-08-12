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

contract StateDiffStorageLayoutTest is DSTest {
    Vm constant vm = Vm(HEVM_ADDRESS);
    SimpleStorage simpleStorage;
    VariousArrays variousArrays;

    function setUp() public {
        simpleStorage = new SimpleStorage();
        variousArrays = new VariousArrays();
    }

    function testSimpleStorageLayout() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Modify storage slots with known positions
        simpleStorage.setValue(42); // Modifies slot 0
        simpleStorage.setOwner(address(this)); // Modifies slot 1
        simpleStorage.setValues(100, 200, 300); // Modifies slots 2, 3, 4

        // Get the state diff as string
        string memory stateDiff = vm.getStateDiff();

        // Get the state diff as JSON and verify it contains the expected structure
        string memory stateDiffJson = vm.getStateDiffJson();

        // The JSON should contain storage layout info for all slots
        // We check the JSON contains expected substrings for the labels and types
        assertContains(stateDiffJson, "\"label\":\"value\"", "Should contain 'value' label");
        assertContains(stateDiffJson, "\"label\":\"owner\"", "Should contain 'owner' label");
        assertContains(stateDiffJson, "\"label\":\"values[0]\"", "Should contain 'values[0]' label");
        assertContains(stateDiffJson, "\"label\":\"values[1]\"", "Should contain 'values[1]' label");
        assertContains(stateDiffJson, "\"label\":\"values[2]\"", "Should contain 'values[2]' label");

        assertContains(stateDiffJson, "\"type\":\"uint256\"", "Should contain uint256 type");
        assertContains(stateDiffJson, "\"type\":\"address\"", "Should contain address type");
        assertContains(stateDiffJson, "\"type\":\"uint256[3]\"", "Should contain uint256[3] type");

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
        assertContains(stateDiffJson, "\"label\":\"numbers[0]\"", "Should contain 'numbers[0]' label");
        assertContains(stateDiffJson, "\"label\":\"numbers[1]\"", "Should contain 'numbers[1]' label");
        assertContains(stateDiffJson, "\"label\":\"numbers[2]\"", "Should contain 'numbers[2]' label");

        assertContains(stateDiffJson, "\"label\":\"addresses[0]\"", "Should contain 'addresses[0]' label");
        assertContains(stateDiffJson, "\"label\":\"addresses[1]\"", "Should contain 'addresses[1]' label");

        assertContains(stateDiffJson, "\"label\":\"flags[0]\"", "Should contain 'flags[0]' label");

        assertContains(stateDiffJson, "\"label\":\"hashes[0]\"", "Should contain 'hashes[0]' label");
        assertContains(stateDiffJson, "\"label\":\"hashes[1]\"", "Should contain 'hashes[1]' label");

        // Verify types are correctly identified
        assertContains(stateDiffJson, "\"type\":\"uint256[3]\"", "Should contain uint256[3] type");
        assertContains(stateDiffJson, "\"type\":\"address[2]\"", "Should contain address[2] type");
        assertContains(stateDiffJson, "\"type\":\"bool[5]\"", "Should contain bool[5] type");
        assertContains(stateDiffJson, "\"type\":\"bytes32[2]\"", "Should contain bytes32[2] type");

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
        assertContains(stateDiffJson, "\"previousValue\":", "JSON should contain previousValue field");
        assertContains(stateDiffJson, "\"newValue\":", "JSON should contain newValue field");
        assertContains(stateDiffJson, "\"label\":", "JSON should contain label field");
        assertContains(stateDiffJson, "\"type\":", "JSON should contain type field");
        assertContains(stateDiffJson, "\"offset\":", "JSON should contain offset field");
        assertContains(stateDiffJson, "\"slot\":", "JSON should contain slot field");

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
