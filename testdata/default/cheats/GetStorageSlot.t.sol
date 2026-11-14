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
    mapping(address => mapping(uint256 => bool)) public approvals; // slot 4
    uint256[] public items; // slot 5
    uint128[] public smallItems; // slot 6
    uint256[3] public fixedItems; // slots 7..9
    string public name; // slot 10

    /// @custom:storage-location erc7201:example.storage.SlotLayout
    struct NamespacedStorage {
        mapping(address => uint256) credits;
        uint64 version;
        bool locked;
    }

    function _namespaced() internal pure returns (NamespacedStorage storage $) {
        bytes32 slot =
            keccak256(abi.encode(uint256(keccak256("example.storage.SlotLayout")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }

    function setPacked(uint64 _a, uint64 _b) public {
        a = _a;
        b = _b;
    }

    function setBalance(address account, uint256 amount) public {
        balances[account] = amount;
    }

    function setApproval(address account, uint256 id, bool approved) public {
        approvals[account][id] = approved;
    }

    function pushItem(uint256 item) public {
        items.push(item);
    }

    function pushSmallItem(uint128 item) public {
        smallItems.push(item);
    }

    function setCredits(address account, uint256 amount) public {
        _namespaced().credits[account] = amount;
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
        assertLocation("fixedItems", 7, 0, 96);
        assertLocation("name", 10, 0, 32);
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

    function testMappings() public {
        assertLocation("balances", 3, 0, 32);
        assertLocation("approvals", 4, 0, 32);

        address user = address(0xBEEF);
        target.setBalance(user, 100);
        target.setApproval(user, 7, true);

        (uint256 balancesSlot,,) = vm.getStorageSlot(address(target), "balances");
        bytes32 balanceSlot = keccak256(abi.encode(user, balancesSlot));
        assertEq(uint256(vm.load(address(target), balanceSlot)), 100);

        vm.store(address(target), balanceSlot, bytes32(uint256(250)));
        assertEq(target.balances(user), 250);

        (uint256 approvalsSlot,,) = vm.getStorageSlot(address(target), "approvals");
        bytes32 innerSlot = keccak256(abi.encode(uint256(7), keccak256(abi.encode(user, approvalsSlot))));
        assertEq(uint256(vm.load(address(target), innerSlot)), 1);
    }

    function testDynamicArrays() public {
        assertLocation("items", 5, 0, 32);
        assertLocation("smallItems", 6, 0, 32);

        target.pushItem(11);
        target.pushItem(22);
        target.pushSmallItem(3);
        target.pushSmallItem(4);
        target.pushSmallItem(5);

        // The base slot holds the length, and elements start at keccak256(baseSlot).
        (uint256 itemsSlot,,) = vm.getStorageSlot(address(target), "items");
        assertEq(uint256(vm.load(address(target), bytes32(itemsSlot))), 2);
        uint256 itemsData = uint256(keccak256(abi.encode(itemsSlot)));
        assertEq(uint256(vm.load(address(target), bytes32(itemsData))), 11);
        assertEq(uint256(vm.load(address(target), bytes32(itemsData + 1))), 22);

        vm.store(address(target), bytes32(itemsData + 1), bytes32(uint256(33)));
        assertEq(target.items(1), 33);

        // Two uint128 elements share a slot, so the third one lands in the second data slot.
        (uint256 smallSlot,,) = vm.getStorageSlot(address(target), "smallItems");
        assertEq(uint256(vm.load(address(target), bytes32(smallSlot))), 3);
        uint256 smallData = uint256(keccak256(abi.encode(smallSlot)));
        assertEq(uint256(vm.load(address(target), bytes32(smallData))), (uint256(4) << 128) | 3);
        assertEq(uint256(vm.load(address(target), bytes32(smallData + 1))), 5);
    }

    function testNamespacedVariables() public {
        uint256 root = uint256(
            keccak256(abi.encode(uint256(keccak256("example.storage.SlotLayout")) - 1)) & ~bytes32(uint256(0xff))
        );
        assertLocation("example.storage.SlotLayout.credits", root, 0, 32);
        assertLocation("example.storage.SlotLayout.version", root + 1, 0, 8);
        assertLocation("example.storage.SlotLayout.locked", root + 1, 8, 1);

        address user = address(0xCAFE);
        target.setCredits(user, 9);
        bytes32 creditSlot = keccak256(abi.encode(user, root));
        assertEq(uint256(vm.load(address(target), creditSlot)), 9);
    }

    function testVariableNameIsCaseInsensitive() public {
        assertLocation("BALANCES", 3, 0, 32);
    }

    function testUnknownVariable() public {
        vm._expectCheatcodeRevert("variable 'missing' not found in storage layout");
        vm.getStorageSlot(address(target), "missing");
    }
}
