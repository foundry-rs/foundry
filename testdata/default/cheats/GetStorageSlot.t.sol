// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract StorageSlotLayout {
    uint256 public value; // slot 0
    uint64 public a; // slot 1, offset 0
    uint64 public b; // slot 1, offset 8
    address public owner; // slot 2, offset 0
    bool public flag; // slot 2, offset 20
    mapping(address => uint256) public balances; // slot 3
    uint256[] public items; // slot 4
    uint256[3] public fixedItems; // slots 5..7
    string public name; // slot 8
    uint256 public A; // slot 9, differs from `a` only by case

    /// @custom:storage-location erc7201:example.storage.SlotLayout
    struct NamespacedStorage {
        mapping(address => uint256) credits;
        uint64 version;
        bool locked;
    }

    function setPacked(uint64 _a, uint64 _b) public {
        a = _a;
        b = _b;
    }
}

contract GetStorageSlotTest is Test {
    StorageSlotLayout target;

    function setUp() public {
        target = new StorageSlotLayout();
    }

    function assertLocation(string memory variable, uint256 slot, uint256 offset, uint256 numberOfBytes) internal {
        (uint256 s, uint256 o, uint256 n) = vm.getStorageSlot(address(target), variable);
        assertEq(s, slot, string.concat(variable, ": slot"));
        assertEq(o, offset, string.concat(variable, ": offset"));
        assertEq(n, numberOfBytes, string.concat(variable, ": numberOfBytes"));
    }

    function testPlainVariables() public {
        assertLocation("value", 0, 0, 32);
        assertLocation("fixedItems", 5, 0, 96);
        assertLocation("name", 8, 0, 32);
    }

    function testPackedVariables() public {
        assertLocation("a", 1, 0, 8);
        assertLocation("b", 1, 8, 8);
        assertLocation("owner", 2, 0, 20);
        assertLocation("flag", 2, 20, 1);

        // Overwrite only `b` using the reported offset and size, leaving `a` untouched.
        target.setPacked(1, 2);
        (uint256 slot, uint256 offset, uint256 numberOfBytes) = vm.getStorageSlot(address(target), "b");
        uint256 mask = ((1 << (numberOfBytes * 8)) - 1) << (offset * 8);
        uint256 word = uint256(vm.load(address(target), bytes32(slot)));
        word = (word & ~mask) | (uint256(42) << (offset * 8));
        vm.store(address(target), bytes32(slot), bytes32(word));

        assertEq(target.a(), 1);
        assertEq(target.b(), 42);
    }

    function testMappingsAndDynamicArrays() public {
        assertLocation("balances", 3, 0, 32);
        assertLocation("items", 4, 0, 32);
    }

    function testNamespacedVariables() public {
        uint256 root = uint256(
            keccak256(abi.encode(uint256(keccak256("example.storage.SlotLayout")) - 1)) & ~bytes32(uint256(0xff))
        );
        assertLocation("example.storage.SlotLayout.credits", root, 0, 32);
        assertLocation("example.storage.SlotLayout.version", root + 1, 0, 8);
        assertLocation("example.storage.SlotLayout.locked", root + 1, 8, 1);
    }

    function testVariableNameMatching() public {
        // An exact match wins over a case-insensitive one.
        assertLocation("a", 1, 0, 8);
        assertLocation("A", 9, 0, 32);
        assertLocation("BALANCES", 3, 0, 32);
    }

    function testUnknownVariable() public {
        vm._expectCheatcodeRevert("variable 'missing' not found in storage layout");
        vm.getStorageSlot(address(target), "missing");
    }

    function testNoStorageLayout() public {
        vm._expectCheatcodeRevert(
            "Storage layout not available for contract at 0x000000000000000000000000000000000000dEaD. Try compiling contracts with `--extra-output storageLayout`"
        );
        vm.getStorageSlot(address(0xdEaD), "value");
    }
}
