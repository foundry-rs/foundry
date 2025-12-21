// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract StorageContract {
    // Simple variables - 1 slot each
    uint256 public value; // Slot 0
    address public owner; // Slot 1

    // Fixed array - 3 consecutive slots
    uint256[3] public numbers; // Slots 2, 3, 4

    // Bytes variables
    bytes public shortBytes; // Slot 5 (less than 32 bytes)
    mapping(address => uint256) public balances; // Slot 6 Inserted in between to make sure we can still properly identify the bytes
    bytes public longBytes; // Slot 7 (32+ bytes, will use multiple slots)

    // String variables
    string public shortString; // Slot 8 (less than 32 bytes)
    string public longString; // Slot 9 (32+ bytes, will use multiple slots)

    function setShortBytes(bytes memory _data) public {
        shortBytes = _data;
    }

    function setLongBytes(bytes memory _data) public {
        longBytes = _data;
    }

    function setShortString(string memory _str) public {
        shortString = _str;
    }

    function setLongString(string memory _str) public {
        longString = _str;
    }

    function setNumbers(uint256 a, uint256 b, uint256 c) public {
        numbers[0] = a;
        numbers[1] = b;
        numbers[2] = c;
    }
}

contract GetStorageSlotsTest is Test {
    StorageContract storageContract;

    function setUp() public {
        storageContract = new StorageContract();
    }

    function testGetStorageSlots() public {
        // Test 1: Simple variable
        uint256[] memory slots = vm.getStorageSlots(address(storageContract), "value");
        assertEq(slots.length, 1);
        assertEq(slots[0], 0);

        // Test 2: Fixed array (should return 3 consecutive slots)
        slots = vm.getStorageSlots(address(storageContract), "numbers");
        assertEq(slots.length, 3);
        assertEq(slots[0], 2);
        assertEq(slots[1], 3);
        assertEq(slots[2], 4);

        // Test 3: Short bytes (less than 32 bytes)
        storageContract.setShortBytes(hex"deadbeef");
        slots = vm.getStorageSlots(address(storageContract), "shortBytes");
        assertEq(slots.length, 1);
        assertEq(slots[0], 5);

        // Test 4: Long bytes (100 bytes = 4 slots needed)
        bytes memory longData = new bytes(100);
        for (uint256 i = 0; i < 100; i++) {
            longData[i] = bytes1(uint8(i));
        }
        storageContract.setLongBytes(longData);

        slots = vm.getStorageSlots(address(storageContract), "longBytes");
        // Should return 5 slots: 1 base slot + 4 data slots
        assertEq(slots.length, 5);
        assertEq(slots[0], 7); // Base slot

        // Data slots start at keccak256(base_slot)
        uint256 dataStart = uint256(keccak256(abi.encode(uint256(7))));
        assertEq(slots[1], dataStart);
        assertEq(slots[2], dataStart + 1);
        assertEq(slots[3], dataStart + 2);
        assertEq(slots[4], dataStart + 3);
    }
}
