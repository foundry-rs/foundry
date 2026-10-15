// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract SlotNumberTest is Test {
    address constant SLOT_READER = address(0x1234);

    function setUp() public {
        vm.setEvmVersion("amsterdam");
        // SLOTNUM; MSTORE(0); RETURN(0, 32).
        vm.etch(SLOT_READER, hex"4b60005260206000f3");
    }

    function readSlotNumber() internal view returns (uint256) {
        (bool success, bytes memory result) = SLOT_READER.staticcall("");
        require(success, "SLOTNUM failed");
        return abi.decode(result, (uint256));
    }

    function testRollSlot() public {
        uint256 number = vm.getBlockNumber();
        uint256 timestamp = vm.getBlockTimestamp();
        assertEq(vm.getSlotNumber(), readSlotNumber());
        vm.rollSlot(100);
        assertEq(vm.getSlotNumber(), 100);
        assertEq(readSlotNumber(), 100);
        vm.rollSlot(0);
        assertEq(vm.getSlotNumber(), 0);
        assertEq(readSlotNumber(), 0);
        vm.rollSlot(type(uint64).max);
        assertEq(vm.getSlotNumber(), type(uint64).max);
        assertEq(readSlotNumber(), type(uint64).max);
        assertEq(vm.getBlockNumber(), number);
        assertEq(vm.getBlockTimestamp(), timestamp);
    }

    function testRollSlotFuzzed(uint64 first, uint64 second) public {
        vm.rollSlot(first);
        assertEq(vm.getSlotNumber(), first);
        assertEq(readSlotNumber(), first);
        vm.rollSlot(second);
        assertEq(vm.getSlotNumber(), second);
        assertEq(readSlotNumber(), second);
    }

    function testRollSlotSnapshot() public {
        vm.rollSlot(42);
        uint256 snapshot = vm.snapshotState();
        vm.rollSlot(100);
        assertTrue(vm.revertToState(snapshot));
        assertEq(vm.getSlotNumber(), 42);
        assertEq(readSlotNumber(), 42);
    }

    function testSlotNumberBeforeAmsterdam() public {
        vm.setEvmVersion("osaka");
        vm._expectCheatcodeRevert(
            "vm.rollSlot: `rollSlot` is not supported before the Amsterdam hard fork; see EIP-7843: https://eips.ethereum.org/EIPS/eip-7843"
        );
        vm.rollSlot(1);
        vm._expectCheatcodeRevert(
            "vm.getSlotNumber: `getSlotNumber` is not supported before the Amsterdam hard fork; see EIP-7843: https://eips.ethereum.org/EIPS/eip-7843"
        );
        vm.getSlotNumber();
    }
}
