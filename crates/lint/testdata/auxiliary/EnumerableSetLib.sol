// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Imported by EnumerableLoopRemoval.sol under an alias: the declared library name is what the
// detector matches, not the name at the call site.
library EnumerableSet {
    struct AddressSet {
        address[] _values;
        mapping(address => uint256) _positions;
    }

    function at(AddressSet storage set, uint256 index) internal view returns (address) {
        return set._values[index];
    }

    function remove(AddressSet storage set, address value) internal returns (bool) {
        uint256 position = set._positions[value];
        if (position == 0) return false;
        uint256 lastIndex = set._values.length - 1;
        if (position - 1 != lastIndex) {
            address lastValue = set._values[lastIndex];
            set._values[position - 1] = lastValue;
            set._positions[lastValue] = position;
        }
        set._values.pop();
        delete set._positions[value];
        return true;
    }

    function length(AddressSet storage set) internal view returns (uint256) {
        return set._values.length;
    }
}
