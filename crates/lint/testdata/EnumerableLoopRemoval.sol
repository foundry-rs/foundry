//@compile-flags: --only-lint enumerable-loop-removal
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {EnumerableSet as ES} from "./auxiliary/EnumerableSetLib.sol";

// Tests for `enumerable-loop-removal`: EnumerableSet removal is swap-and-pop, so calling
// `remove` inside a loop that also iterates the same set with `at` at a varying index skips
// elements or reads out-of-bounds indices. Calls resolve through the type checker, so the
// method-call form and the library-qualified form are both covered. Clean patterns:
// collect-then-remove (the `remove` loop has no `at`), draining at a literal index, `at` on
// a different set instance, and same-name functions that are not EnumerableSet's.

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
    using Helpers for EnumerableSet.AddressSet;

    struct SetPair {
        EnumerableSet.AddressSet first;
        EnumerableSet.AddressSet second;
    }

    EnumerableSet.AddressSet internal holders;
    EnumerableSet.AddressSet internal others;
    EnumerableSet.UintSet internal ids;
    CustomSet.Bag internal bag;
    SetPair internal pair;
    mapping(uint256 => EnumerableSet.AddressSet) internal keyed;
    address internal pending;
    uint256 internal cursor;

    function hasMoreToVisit() internal view returns (bool) {
        return cursor < holders.length();
    }

    function removeWhileIteratingFor() public {
        // `remove` shrinks the set while `at(i)` walks it: elements are skipped.
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeWhileIteratingWhile() public {
        while (holders.length() > 0) {
            // draining at the literal index 0 is safe: the swap refills the read position
            address value = holders.at(0);
            holders.remove(value);
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
        // The `at` walks a different EnumerableSet instance: removing from `holders`
        // cannot corrupt the iteration over `others`.
        for (uint256 i = 0; i < others.length(); i++) {
            holders.remove(others.at(i));
        }
    }

    function removeQualifiedForm() public {
        // The library-qualified form of the unsafe pattern: same set, varying index.
        for (uint256 i = 0; i < EnumerableSet.length(holders); i++) {
            EnumerableSet.remove(holders, EnumerableSet.at(holders, i)); //~WARN: EnumerableSet
        }
    }

    function removeMixedForms() public {
        // The two call forms mixed on the same set corrupt the iteration all the same.
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(EnumerableSet.at(holders, i)); //~WARN: EnumerableSet
        }
    }

    ES.AddressSet internal imported;

    function removeAliasedImport() public {
        // An import alias renames the call site, not the declared library the calls
        // resolve to.
        for (uint256 i = 0; i < ES.length(imported); i++) {
            ES.remove(imported, ES.at(imported, i)); //~WARN: EnumerableSet
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

    // The alias may stand on the side of the read as well as on the side of the removal.
    function removeThroughStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = holders;
        for (uint256 i = 0; i < alias_.length(); i++) {
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeThroughStorageAliasWhileIteratingTheState() public {
        EnumerableSet.AddressSet storage alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            alias_.remove(holders.at(i)); //~WARN: EnumerableSet
        }
    }

    // The loop ends on the removal, so the shrunk set is never read again.
    function removeAndBreak(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == target) {
                holders.remove(target);
                break;
            }
        }
    }

    function removeAndReturn(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == target) {
                holders.remove(target);
                return;
            }
        }
    }

    // The `break` ends the inner loop, and the outer one reads the set again.
    function removeAndBreakInnerLoop() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            for (uint256 j = 0; j < 1; j++) {
                holders.remove(pending); //~WARN: EnumerableSet
                break;
            }
            pending = holders.at(i);
        }
    }

    // A `continue` comes round again, so the removal still corrupts the iteration.
    function removeAndContinue(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == target) {
                holders.remove(target); //~WARN: EnumerableSet
                continue;
            }
        }
    }

    // Each inner loop reads the set anew: the collecting one has ended before the removing one
    // starts, and the outer loop reads nothing itself.
    function collectThenRemoveInsideOuterLoop() public {
        for (uint256 round = 0; round < 1; round++) {
            address[] memory pendingRemovals = new address[](holders.length());
            for (uint256 i = 0; i < holders.length(); i++) {
                pendingRemovals[i] = holders.at(i);
            }
            for (uint256 i = 0; i < pendingRemovals.length; i++) {
                holders.remove(pendingRemovals[i]);
            }
        }
    }

    // Two fields of one struct are two sets, and so are two literal keys of one mapping.
    function removeFromAnotherStructField() public {
        for (uint256 i = 0; i < pair.first.length(); i++) {
            pair.second.remove(pair.first.at(i));
        }
    }

    function removeAtAnotherMappingKey() public {
        for (uint256 i = 0; i < keyed[1].length(); i++) {
            keyed[2].remove(keyed[1].at(i));
        }
    }

    // A key that varies cannot be read, so it may name the iterated set.
    function removeAtVaryingMappingKey(uint256 key) public {
        for (uint256 i = 0; i < keyed[1].length(); i++) {
            keyed[key].remove(keyed[1].at(i)); //~WARN: EnumerableSet
        }
    }

    // The qualified form, written with named arguments.
    function removeNamedArgumentsOnOtherSet() public {
        for (uint256 i = 0; i < others.length(); i++) {
            EnumerableSet.remove({
                value: EnumerableSet.at({index: i, set: others}),
                set: holders
            });
        }
    }

    function removeNamedArgumentsOnSameSet() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            EnumerableSet.remove({ //~WARN: EnumerableSet
                value: EnumerableSet.at({index: i, set: holders}),
                set: holders
            });
        }
    }

    // A named literal index drains at a fixed position, in the method form too.
    function removeMethodFormNamedLiteralIndex() public {
        while (holders.length() > 0) {
            holders.remove(holders.at({index: 0}));
        }
    }

    function removeMethodFormNamedVaryingIndex() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(holders.at({index: i})); //~WARN: EnumerableSet
        }
    }

    // The `at` sits in a nested loop but reads at the outer loop's index. The nested loop
    // removes nothing of its own, so only the outer loop can be reported.
    function removeAtOuterIndexFromInnerLoop() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            for (uint256 j = 0; j < 1; j++) {
                pending = holders.at(i);
            }
            holders.remove(pending); //~WARN: EnumerableSet
        }
    }

    // The inner loop walks the set at its own index and ends before the outer one removes, and
    // the outer loop reads nothing itself.
    function removeAfterInnerLoopWalksAtItsOwnIndex() public {
        for (uint256 round = 0; round < 1; round++) {
            for (uint256 j = 0; j < holders.length(); j++) {
                pending = holders.at(j);
            }
            holders.remove(pending);
        }
    }

    // A nested `while` walks the set at its own index, declared just before it, and the outer
    // loop removes only after that loop ends: collect-then-remove with a `while`, which is safe.
    // The index is the while's own even though a `while` declares it outside itself, because the
    // outer loop does not advance it.
    function removeAfterInnerWhileWalksAtItsOwnIndex() public {
        for (uint256 round = 0; round < 1; round++) {
            uint256 j = 0;
            while (j < holders.length()) {
                pending = holders.at(j);
                j++;
            }
            holders.remove(pending);
        }
    }

    // A nested loop that reads `holders.at(i)` at the OUTER loop's index while also reassigning
    // that index does not take ownership of it: the outer loop still iterates `holders` at `i`,
    // so the removal in the outer body corrupts it. The outer loop's own index is never a nested
    // loop's to claim, even when the nested loop writes it.
    function removeReassigningOuterIndexInInnerLoop() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            for (uint256 j = 0; j < 1; j++) {
                pending = holders.at(i);
                i = i;
            }
            holders.remove(pending); //~WARN: EnumerableSet
        }
    }

    // The same corruption when the outer loop is a `while`: its index `i`, tested in the
    // condition and advanced in the body, is the loop's own even though a `while` declares it
    // outside itself, so a nested loop reassigning `i` does not take it over.
    function removeReassigningOuterWhileIndexInInnerLoop() public {
        uint256 i = 0;
        while (i < holders.length()) {
            for (uint256 j = 0; j < 1; j++) {
                pending = holders.at(i);
                i = i;
            }
            holders.remove(pending); //~WARN: EnumerableSet
            i++;
        }
    }

    // The nested loop both reads and advances the outer index, and the outer loop removes each
    // turn: the index is still the outer loop's, reassigned but not declared inside the body, so
    // the removal corrupts the walk. The condition never has to name the index for this to hold.
    function removeWhereInnerLoopAdvancesOuterIndex() public {
        uint256 i = 0;
        while (i < holders.length()) {
            for (uint256 k = 0; k < 1; k++) {
                pending = holders.at(i);
                i++;
            }
            holders.remove(pending); //~WARN: EnumerableSet
        }
    }

    // A nested loop walks a cursor passed in as a parameter, and the outer loop removes after it:
    // the cursor is the nested loop's, tested and advanced inside it, even though a parameter is
    // declared outside the outer body. Collect-then-remove, so clean.
    function removeAfterInnerLoopWalksParamCursor(uint256 start) public {
        for (uint256 round = 0; round < 1; round++) {
            while (start < holders.length()) {
                pending = holders.at(start);
                start++;
            }
            holders.remove(pending);
        }
    }

    // A hoisted cursor reset each turn is not the outer loop's cadence: a reset does not progress
    // across turns, so the nested loop that advances and tests it owns it.
    function removeAfterInnerLoopReseedsHoistedCursor() public {
        uint256 j;
        for (uint256 round = 0; round < 1; round++) {
            j = 0;
            while (j < holders.length()) {
                pending = holders.at(j);
                j++;
            }
            holders.remove(pending);
        }
    }

    // A plain `if` in the outer body that merely tests a nested loop's cursor is not a
    // termination guard, so it does not make the cursor the outer loop's cadence: the nested
    // loop iterates on `j`, removes after, and stays clean.
    function removeAfterInnerLoopThenTestCursor() public {
        uint256 j;
        for (uint256 round = 0; round < 1; round++) {
            for (j = 0; j < holders.length(); j++) {
                pending = holders.at(j);
            }
            if (j == holders.length()) {
                pending = pending;
            }
            holders.remove(pending);
        }
    }

    // A documented limit: the outer loop's condition is an opaque predicate that hides `cursor`,
    // advanced only inside a nested loop that iterates on `k`. `cursor` names neither a condition
    // nor a direct progression of the outer loop, so this corruption goes unreported. Reading
    // `hasMoreToVisit`'s body would take an interprocedural pass this detector does not run.
    function removeWithOpaqueConditionCursorIsMissed() public {
        while (hasMoreToVisit()) {
            for (uint256 k = 0; k < 1; k++) {
                pending = holders.at(cursor);
                cursor++;
            }
            holders.remove(pending);
        }
    }

    uint256 internal constant FIRST = 0;

    // A fixed index is a safe drain wherever its value is settled at compile time: a named
    // `constant` or a cast of a literal reads position zero, which swap-and-pop keeps refilling,
    // just as the bare literal `at(0)` does.
    function removeAtNamedConstantIndex() public {
        while (holders.length() > 0) {
            holders.remove(holders.at(FIRST));
        }
    }

    function removeAtCastLiteralIndex() public {
        while (holders.length() > 0) {
            holders.remove(holders.at(uint256(0)));
        }
    }

    // Constant arithmetic is not folded, so a fixed index written as an expression is treated as
    // varying and reported: a conservative report on a drain that is in fact safe.
    function removeAtConstantArithmeticIndex() public {
        while (holders.length() > 0) {
            holders.remove(holders.at(0 + 0)); //~WARN: EnumerableSet
        }
    }

    // The function binds the reference again, so its initializer no longer says which set it
    // names: it may be any of them.
    function removeThroughReassignedStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = others;
        alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // The same unreadable reference, read against a set the loop does not remove from. Matching it
    // against every set reports a removal that is in fact safe, rather than missing an unsafe one.
    function removeAnotherSetThroughReassignedStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = others;
        alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            others.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // Both clauses of the `try` leave the function, so the shrunk set is never read again.
    function removeAndReturnFromEveryTryClause() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            try this.probe() {
                holders.remove(value);
                return;
            } catch {
                return;
            }
        }
    }

    // The `catch` falls through, so the loop comes round again on the shrunk set.
    function removeAndFallThroughACatch() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            try this.probe() {
                holders.remove(value); //~WARN: EnumerableSet
                return;
            } catch {}
        }
    }

    function probe() external {}

    function customSetIsNotEnumerableSet() public {
        // Same `at`/`remove` names on a non-EnumerableSet type: out of scope.
        for (uint256 i = 0; i < bag.length(); i++) {
            if (bag.at(i) == address(0)) bag.remove(i);
        }
    }

    function helperRemoveIsNotEnumerable() public {
        // A user library function named `remove` attached to the set type: the call resolves
        // to the helper, not to EnumerableSet's swap-and-pop, so it is out of scope.
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == address(0)) holders.remove(i);
        }
    }
}

// A user library attached to the EnumerableSet type, with its own `remove` overload.
library Helpers {
    function remove(EnumerableSet.AddressSet storage, uint256 index) internal pure {
        index += 1;
    }
}
