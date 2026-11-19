// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.35;

import "utils/Test.sol";

/// Shaped like OpenZeppelin's `EnumerableSet`: the mapping is a member of a struct nested in another.
library Sets {
    struct Set {
        bytes32[] _values;
        mapping(bytes32 => uint256) _positions;
    }

    struct AddressSet {
        Set _inner;
    }

    /// Writes only `_positions`; the `_values` length at the set's base slot is not part of this test.
    function setPosition(AddressSet storage set, address value, uint256 position) internal {
        set._inner._positions[bytes32(uint256(uint160(value)))] = position;
    }
}

contract StructMappings {
    struct Balances {
        uint256 total;
        mapping(address => uint256) byAccount;
        mapping(bytes32 => uint256) byId;
    }

    uint256 internal pad;
    Balances internal balances;
    Sets.AddressSet internal set;

    function setBalance(address account, bytes32 id, uint256 amount) public {
        balances.byAccount[account] = amount;
        balances.byId[id] = amount;
    }

    function addMember(address member) public {
        Sets.setPosition(set, member, 1);
    }
}

contract NamespacedStructMappings {
    /// @custom:storage-location erc7201:example.storage.Managed
    struct ManagedStorage {
        Sets.AddressSet set;
    }

    function _managed() internal pure returns (ManagedStorage storage $) {
        bytes32 slot =
            keccak256(abi.encode(uint256(keccak256("example.storage.Managed")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }

    function addMember(address member) public {
        Sets.setPosition(_managed().set, member, 1);
    }
}

contract StateDiffStructMappingsTest is Test {
    address internal constant MEMBER = 0x1111111111111111111111111111111111111111;

    function testMappingMemberOfStruct() public {
        StructMappings target = new StructMappings();
        vm.startStateDiffRecording();
        target.setBalance(MEMBER, bytes32(uint256(7)), 100);
        string memory json = vm.getStateDiffJson();

        assertContains(
            json,
            '"label":"balances.byAccount[0x1111111111111111111111111111111111111111]","type":"mapping(address => uint256)"'
        );
        assertContains(
            json,
            '"label":"balances.byId[0x0000000000000000000000000000000000000000000000000000000000000007]","type":"mapping(bytes32 => uint256)"'
        );
        assertContains(json, '"decoded":{"previousValue":"0","newValue":"100"}');
    }

    function testMappingMemberOfNestedStruct() public {
        StructMappings target = new StructMappings();
        vm.startStateDiffRecording();
        target.addMember(MEMBER);
        string memory json = vm.getStateDiffJson();

        assertContains(
            json,
            '"label":"set._inner._positions[0x0000000000000000000000001111111111111111111111111111111111111111]","type":"mapping(bytes32 => uint256)"'
        );
        assertContains(json, '"decoded":{"previousValue":"0","newValue":"1"}');
    }

    function testNamespacedMappingMemberOfNestedStruct() public {
        NamespacedStructMappings target = new NamespacedStructMappings();
        vm.startStateDiffRecording();
        target.addMember(MEMBER);
        string memory json = vm.getStateDiffJson();
        string memory text = vm.getStateDiff();

        assertContains(
            json,
            '"label":"example.storage.Managed.set._inner._positions[0x0000000000000000000000001111111111111111111111111111111111111111]","type":"mapping(bytes32 => uint256)"'
        );
        assertContains(json, '"decoded":{"previousValue":"0","newValue":"1"}');
        assertContains(
            text,
            unicode"(example.storage.Managed.set._inner._positions[0x0000000000000000000000001111111111111111111111111111111111111111], uint256): 0 → 1"
        );
    }

    function assertContains(string memory haystack, string memory needle) internal {
        assertTrue(vm.contains(haystack, needle), needle);
    }
}
