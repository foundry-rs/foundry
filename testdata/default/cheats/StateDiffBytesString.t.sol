// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract BytesStringStorage {
    // Short string (less than 32 bytes)
    string public shortString; // Slot 0

    // Long string (32 bytes or more)
    string public longString; // Slot 1

    // Short bytes
    bytes public shortBytes; // Slot 2

    // Long bytes
    bytes public longBytes; // Slot 3

    // Fixed size bytes
    bytes32 public fixedBytes; // Slot 4

    // Mapping with string values
    mapping(address => string) public userNames; // Slot 5

    function setShortString(string memory _value) public {
        shortString = _value;
    }

    function setLongString(string memory _value) public {
        longString = _value;
    }

    function setShortBytes(bytes memory _value) public {
        shortBytes = _value;
    }

    function setLongBytes(bytes memory _value) public {
        longBytes = _value;
    }

    function setFixedBytes(bytes32 _value) public {
        fixedBytes = _value;
    }

    function setUserName(address user, string memory name) public {
        userNames[user] = name;
    }
}

contract StateDiffBytesStringTest is Test {
    BytesStringStorage bytesStringStorage;

    function setUp() public {
        bytesStringStorage = new BytesStringStorage();
    }

    function testLongStringStorage() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set a long string (32 bytes or more)
        string memory longStr =
            "This is a very long string that exceeds 32 bytes and will be stored differently in Solidity storage";
        bytesStringStorage.setLongString(longStr);

        // Get the state diff as string
        string memory stateDiff = vm.getStateDiff();
        emit log_string("State diff for long string:");
        emit log_string(stateDiff);

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();
        emit log_string("State diff JSON for long string:");
        emit log_string(stateDiffJson);

        // Verify the JSON contains expected fields
        assertTrue(vm.contains(stateDiffJson, '"label":"longString"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"string"'));

        // For long strings, we should see multiple slots being accessed
        // The main slot (slot 1) contains the length
        // The data slots start at keccak256(1)

        // Stop recording
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();
        assertTrue(accesses.length > 0);

        // Verify storage accesses
        uint256 writeCount = 0;
        for (uint256 i = 0; i < accesses.length; i++) {
            if (accesses[i].account == address(bytesStringStorage)) {
                for (uint256 j = 0; j < accesses[i].storageAccesses.length; j++) {
                    if (accesses[i].storageAccesses[j].isWrite) {
                        writeCount++;
                    }
                }
            }
        }
        // Long string should write to multiple slots (main slot + data slots)
        assertTrue(writeCount >= 2);
    }

    function testShortBytesStorage() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set short bytes (less than 32 bytes)
        bytes memory shortData = hex"deadbeef";
        bytesStringStorage.setShortBytes(shortData);

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();
        emit log_string("State diff JSON for short bytes:");
        emit log_string(stateDiffJson);

        // Verify the JSON contains expected fields
        assertTrue(vm.contains(stateDiffJson, '"label":"shortBytes"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes"'));
        assertTrue(vm.contains(stateDiffJson, '"decoded":'));

        // Check the decoded bytes value
        assertTrue(vm.contains(stateDiffJson, '"newValue":"0xdeadbeef"'));

        // Stop recording
        vm.stopAndReturnStateDiff();
    }

    function testLongBytesStorage() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set long bytes (32 bytes or more)
        bytes memory longData = new bytes(100);
        for (uint256 i = 0; i < 100; i++) {
            longData[i] = bytes1(uint8(i));
        }
        bytesStringStorage.setLongBytes(longData);

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();
        emit log_string("State diff JSON for long bytes:");
        emit log_string(stateDiffJson);

        // Verify the JSON contains expected fields
        assertTrue(vm.contains(stateDiffJson, '"label":"longBytes"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes"'));

        // Stop recording
        Vm.AccountAccess[] memory accesses = vm.stopAndReturnStateDiff();

        // Verify multiple slots were written (main slot + data slots)
        uint256 writeCount = 0;
        for (uint256 i = 0; i < accesses.length; i++) {
            if (accesses[i].account == address(bytesStringStorage)) {
                writeCount += accesses[i].storageAccesses.length;
            }
        }
        assertTrue(writeCount >= 2);
    }

    function testFixedBytesStorage() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Set fixed bytes32
        bytes32 fixedData = keccak256("test data");
        bytesStringStorage.setFixedBytes(fixedData);

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();
        emit log_string("State diff JSON for fixed bytes:");
        emit log_string(stateDiffJson);

        // Verify the JSON contains expected fields
        assertTrue(vm.contains(stateDiffJson, '"label":"fixedBytes"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes32"'));
        assertTrue(vm.contains(stateDiffJson, '"decoded":'));

        // Stop recording
        vm.stopAndReturnStateDiff();
    }

    function testMultipleBytesStringChanges() public {
        // Start recording state diffs
        vm.startStateDiffRecording();

        // Make multiple changes
        bytesStringStorage.setShortString("Short");
        bytesStringStorage.setLongString("This is a long string that will use multiple storage slots for data");
        bytesStringStorage.setShortBytes(hex"1234");
        bytesStringStorage.setFixedBytes(bytes32(uint256(0xdeadbeef)));

        // Get the state diff as string
        string memory stateDiff = vm.getStateDiff();
        emit log_string("State diff for multiple changes:");
        emit log_string(stateDiff);

        // Get the state diff as JSON
        string memory stateDiffJson = vm.getStateDiffJson();
        emit log_string("State diff JSON for multiple changes:");
        emit log_string(stateDiffJson);

        // Verify all fields are properly labeled
        assertTrue(vm.contains(stateDiffJson, '"label":"shortString"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"longString"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"shortBytes"'));
        assertTrue(vm.contains(stateDiffJson, '"label":"fixedBytes"'));

        // Verify types are correct
        assertTrue(vm.contains(stateDiffJson, '"type":"string"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes"'));
        assertTrue(vm.contains(stateDiffJson, '"type":"bytes32"'));

        // Stop recording
        vm.stopAndReturnStateDiff();
    }
}
