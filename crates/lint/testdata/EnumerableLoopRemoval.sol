//@compile-flags: --only-lint enumerable-loop-removal
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

// Tests for `enumerable-loop-removal`: EnumerableSet removal is swap-and-pop, so calling
// `remove` inside a loop that also iterates the set with `at` skips elements or reads
// out-of-bounds indices. The safe pattern (collect during the loop, remove after) has a
// `remove` loop without any `at` in it and must stay clean, as must the same method names
// on a type that is not an EnumerableSet.

// Minimal mirror of OpenZeppelin's EnumerableSet: swap-and-pop removal, index access via `at`.
library EnumerableSet {
    struct AddressSet {
        address[] _values;
        mapping(address => uint256) _positions;
    }

    struct UintSet {
        uint256[] _values;
        mapping(uint256 => uint256) _positions;
    }

    function add(AddressSet storage set, address value) internal returns (bool) {
        if (set._positions[value] != 0) return false;
        set._values.push(value);
        set._positions[value] = set._values.length;
        return true;
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

    function at(UintSet storage set, uint256 index) internal view returns (uint256) {
        return set._values[index];
    }

    function remove(UintSet storage set, uint256 value) internal returns (bool) {
        uint256 position = set._positions[value];
        if (position == 0) return false;
        uint256 lastIndex = set._values.length - 1;
        if (position - 1 != lastIndex) {
            uint256 lastValue = set._values[lastIndex];
            set._values[position - 1] = lastValue;
            set._positions[lastValue] = position;
        }
        set._values.pop();
        delete set._positions[value];
        return true;
    }

    function length(UintSet storage set) internal view returns (uint256) {
        return set._values.length;
    }
}

// Same method names and shapes, but not an EnumerableSet: must never fire.
library CustomSet {
    struct Bag {
        address[] _values;
    }

    function at(Bag storage bag, uint256 index) internal view returns (address) {
        return bag._values[index];
    }

    function remove(Bag storage bag, uint256 index) internal {
        bag._values[index] = bag._values[bag._values.length - 1];
        bag._values.pop();
    }

    function length(Bag storage bag) internal view returns (uint256) {
        return bag._values.length;
    }
}

contract EnumerableLoopRemoval {
    using EnumerableSet for EnumerableSet.AddressSet;
    using EnumerableSet for EnumerableSet.UintSet;
    using CustomSet for CustomSet.Bag;

    EnumerableSet.AddressSet internal holders;
    EnumerableSet.AddressSet internal others;
    EnumerableSet.UintSet internal ids;
    CustomSet.Bag internal bag;

    function removeWhileIteratingFor() public {
        // `remove` shrinks the set while `at(i)` walks it: elements are skipped.
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeWhileIteratingWhile() public {
        while (holders.length() > 0) {
            address value = holders.at(0);
            holders.remove(value); //~WARN: EnumerableSet
        }
    }

    function removeWhileIteratingDoWhile() public {
        uint256 i = 0;
        do {
            address value = holders.at(i);
            holders.remove(value); //~WARN: EnumerableSet
            i++;
        } while (i < holders.length());
    }

    function removeWithAtOnOtherSet() public {
        // The `at` walks another EnumerableSet instance, but the loop still mixes
        // index iteration and removal.
        for (uint256 i = 0; i < others.length(); i++) {
            holders.remove(others.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeUintSet() public {
        for (uint256 i = 0; i < ids.length(); i++) {
            ids.remove(ids.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeInnerLoopAtOuterLoop() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            for (uint256 j = 0; j < 2; j++) {
                holders.remove(value); //~WARN: EnumerableSet
            }
        }
    }

    function collectThenRemove() public {
        // Safe pattern: iterate with `at`, collect, then remove in a second loop
        // that contains no `at`.
        address[] memory toRemove = new address[](holders.length());
        for (uint256 i = 0; i < holders.length(); i++) {
            toRemove[i] = holders.at(i);
        }
        for (uint256 i = 0; i < toRemove.length; i++) {
            holders.remove(toRemove[i]);
        }
    }

    function atOnlyInLoop() public view returns (uint256 count) {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) != address(0)) count++;
        }
    }

    function removeOutsideLoop(address value) public {
        holders.remove(value);
    }

    function customSetIsNotEnumerableSet() public {
        // Same `at`/`remove` names on a non-EnumerableSet type: out of scope.
        for (uint256 i = 0; i < bag.length(); i++) {
            if (bag.at(i) == address(0)) bag.remove(i);
        }
    }
}
