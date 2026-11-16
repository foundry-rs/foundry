// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.35;

import "utils/Test.sol";

/// Shaped like OpenZeppelin's `EnumerableSet`: `AddressSet`'s sole member is a two-slot struct.
library EnumerableSet {
    struct Set {
        bytes32[] _values;
        mapping(bytes32 => uint256) _positions;
    }

    struct AddressSet {
        Set _inner;
    }

    function add(AddressSet storage set, address value) internal {
        bytes32 key = bytes32(uint256(uint160(value)));
        set._inner._values.push(key);
        set._inner._positions[key] = set._inner._values.length;
    }
}

contract PlainSet {
    using EnumerableSet for EnumerableSet.AddressSet;

    uint256 internal count;
    EnumerableSet.AddressSet internal members;

    function add(address value) public {
        members.add(value);
    }
}

contract NamespacedSet {
    using EnumerableSet for EnumerableSet.AddressSet;

    /// @custom:storage-location erc7201:example.storage.Members
    struct MembersStorage {
        uint256 count;
        EnumerableSet.AddressSet members;
    }

    function _members() internal pure returns (MembersStorage storage $) {
        bytes32 slot =
            keccak256(abi.encode(uint256(keccak256("example.storage.Members")) - 1)) & ~bytes32(uint256(0xff));
        assembly {
            $.slot := slot
        }
    }

    function add(address value) public {
        _members().members.add(value);
    }
}

contract StateDiffNestedStructTest is Test {
    function testPlainNestedStructBaseSlot() public {
        PlainSet set = new PlainSet();
        vm.startStateDiffRecording();
        set.add(address(0xBEEF));
        string memory json = vm.getStateDiffJson();

        // The set's base slot is the inner array's length, not an empty struct.
        assertTrue(!vm.contains(json, '"decoded":{"previousValue":"()","newValue":"()"}'));
        assertTrue(
            vm.contains(
                json,
                '"label":"members._inner._values","type":"bytes32[]","offset":0,"slot":"1","decoded":{"previousValue":"0","newValue":"1"}'
            )
        );
    }

    function testNamespacedNestedStructBaseSlot() public {
        NamespacedSet set = new NamespacedSet();
        vm.startStateDiffRecording();
        set.add(address(0xBEEF));
        string memory json = vm.getStateDiffJson();

        // `members` is the namespace's second slot.
        uint256 slot = uint256(
            keccak256(abi.encode(uint256(keccak256("example.storage.Members")) - 1)) & ~bytes32(uint256(0xff))
        ) + 1;
        assertTrue(!vm.contains(json, '"decoded":{"previousValue":"()","newValue":"()"}'));
        assertTrue(
            vm.contains(
                json,
                string.concat(
                    '"label":"example.storage.Members.members._inner._values","type":"bytes32[]","offset":0,"slot":"',
                    vm.toString(slot),
                    '","decoded":{"previousValue":"0","newValue":"1"}'
                )
            )
        );
    }
}
