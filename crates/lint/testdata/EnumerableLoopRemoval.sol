//@compile-flags: --only-lint enumerable-loop-removal
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {EnumerableSet as ES} from "./auxiliary/EnumerableSetLib.sol";

// Tests for `enumerable-loop-removal`: EnumerableSet removal is swap-and-pop, so calling
// `remove` inside a loop that also iterates the same set with `at` at an index the loop
// advances skips elements or reads out-of-bounds indices. Calls resolve through the type
// checker, so the method-call form and the library-qualified form are both covered. Clean
// patterns: collect-then-remove (the `remove` loop has no `at`), draining at an index the loop
// never moves, walking downward only, removing and leaving the loop, `at` on a different set
// instance, and same-name functions that are not EnumerableSet's.

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
    uint256 internal position;

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

    // `remove` answers true exactly when it took the value out, so a removal deciding an
    // exiting branch mutates only on the path that leaves: nothing iterates after the set
    // shrank, and the continuing path left it untouched.
    function removeAsExitingCondition(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            if (holders.remove(target)) break;
        }
    }

    // Another operand rides along: execution can pass the exit after the set shrank.
    function removeConditionCarriesAnotherOperand(address target, bool flag) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            if (holders.remove(target) && flag) break; //~WARN: EnumerableSet
        }
    }

    // The branch the mutating answer takes does not leave, so the iteration continues on the
    // shrunk set.
    function removeAsConditionWithoutExit(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            if (holders.remove(target)) { //~WARN: EnumerableSet
                pending = address(0);
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

    // A literal, a cast of one, or a named `constant` never names the loop's cadence: the read
    // drains position zero, which swap-and-pop keeps refilling, just as the bare literal
    // `at(0)` does.
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

    // Constant arithmetic names no variable at all, so it is not the loop's cadence either:
    // the drain reads the same way as the folded literal would.
    function removeAtConstantArithmeticIndex() public {
        while (holders.length() > 0) {
            holders.remove(holders.at(0 + 0));
        }
    }

    // A cursor the loop never advances is not its cadence either: the read drains the position
    // that swap-and-pop keeps refilling, and the loop ends when the length comes down to it.
    function removeAtStationaryCursor() public {
        while (holders.length() > position) {
            holders.remove(holders.at(position));
        }
    }

    // The same shape with the cursor advanced by the loop walks the set, and the removal skips
    // the elements swapped into the slots behind the cursor.
    function removeAtAdvancingStateCursor() public {
        while (cursor < holders.length()) {
            holders.remove(holders.at(cursor)); //~WARN: EnumerableSet
            cursor++;
        }
    }

    // A cadence only stepped downward drains from the top: removing at the walk's own index
    // moves the tail into the slot just read, which a descending walk never returns to.
    function removeAtDescendingIndex() public {
        for (uint256 i = holders.length(); i > 0; i--) {
            holders.remove(holders.at(i - 1));
        }
    }

    // The downward step written as a plain assignment reads the same way.
    function removeAtDescendingAssignedIndex() public {
        uint256 i = holders.length();
        while (i > 0) {
            i = i - 1;
            holders.remove(holders.at(i));
        }
    }

    // One upward step anywhere breaks the downward walk: the direction is no longer known, so
    // the index is kept as the loop's cadence.
    function removeAtDescendingIndexAlsoSteppedUp(bool skipTwo) public {
        for (uint256 i = holders.length(); i > 0; i--) {
            holders.remove(holders.at(i - 1)); //~WARN: EnumerableSet
            if (skipTwo) {
                i += 2;
            }
        }
    }

    // A straight-line copy of the cadence walks the loop exactly as the cadence does.
    function removeAtCopiedIndex() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            uint256 idx = i;
            holders.remove(holders.at(idx)); //~WARN: EnumerableSet
        }
    }

    // An index derived from the cadence may hold any position as the loop turns, so it is read
    // as walking the loop even when its arithmetic happens to walk downward: a conservative
    // report on a mirror that drains safely.
    function removeAtIndexComputedFromCadence() public {
        uint256 count = holders.length();
        for (uint256 i = 0; i < count; i++) {
            uint256 mirrored = count - 1 - i;
            holders.remove(holders.at(mirrored)); //~WARN: EnumerableSet
        }
    }

    // A copy of a cursor the loop never moves carries no cadence either.
    function removeAtCopyOfStationaryCursor() public {
        while (holders.length() > position) {
            uint256 idx = position;
            holders.remove(holders.at(idx));
        }
    }

    // The straight-line rebinding stands where the loop runs: `alias_` names `holders` here,
    // the very set the loop removes from.
    function removeThroughReassignedStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = others;
        alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // The same reference read against the set it stopped naming: at the loop `alias_` is
    // `holders`, so removing from `others` touches nothing the loop iterates.
    function removeAnotherSetThroughReassignedStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = others;
        alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            others.remove(alias_.at(i));
        }
    }

    // The reference is rebound only after the loop: where the loop runs it still names
    // `others`, so removing from `holders` corrupts nothing the loop iterates.
    function removeThroughAliasReboundAfterTheLoop() public {
        EnumerableSet.AddressSet storage alias_ = others;
        for (uint256 i = 0; i < alias_.length(); i++) {
            holders.remove(alias_.at(i));
        }
        alias_ = holders;
    }

    // Rebound under a condition, the reference has no one binding where the loop reads it, so
    // it may name the removed set.
    function removeThroughConditionallyReboundAlias(bool swap) public {
        EnumerableSet.AddressSet storage alias_ = others;
        if (swap) {
            alias_ = holders;
        }
        for (uint256 i = 0; i < alias_.length(); i++) {
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // Rebound inside the loop itself, the reference may name either set on any turn.
    function removeThroughAliasReboundInsideTheLoop(bool swap) public {
        EnumerableSet.AddressSet storage alias_ = others;
        for (uint256 i = 0; i < holders.length(); i++) {
            if (swap) {
                alias_ = holders;
            }
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // A reference bound to another reference takes what that one named right then: the later
    // rebinding of `source` does not reach back, so `alias_` still walks `others`, the set
    // being removed from.
    function removeThroughAliasBoundToEarlierAlias() public {
        EnumerableSet.AddressSet storage source = others;
        EnumerableSet.AddressSet storage alias_ = source;
        source = holders;
        for (uint256 i = 0; i < alias_.length(); i++) {
            others.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // A tuple-destructured reference never got a binding the walk can read and carries no
    // initializer, so it may name the removed set, the same way a single unreadable reference
    // does.
    function removeThroughTupleBoundAlias() public {
        (EnumerableSet.AddressSet storage first,) = twoSets();
        for (uint256 i = 0; i < first.length(); i++) {
            holders.remove(first.at(i)); //~WARN: EnumerableSet
        }
    }

    function twoSets()
        internal
        view
        returns (EnumerableSet.AddressSet storage, EnumerableSet.AddressSet storage)
    {
        return (holders, holders);
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

    // The removal on the success path is followed by that clause's own `return`; the `catch`
    // falls through only when the tried call reverted, before anything was removed, so no path
    // iterates on a shrunk set.
    function removeAndFallThroughACatch() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            try this.probe() {
                holders.remove(value);
                return;
            } catch {}
        }
    }

    // A removal inside the `catch` that falls through comes round on the shrunk set.
    function removeInsideFallingCatch() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            try this.probe() {
                return;
            } catch {
                holders.remove(value); //~WARN: EnumerableSet
            }
        }
    }

    // A success clause that falls through iterates after the removal all the same.
    function removeAndFallThroughSuccessClause() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            try this.probe() {
                holders.remove(value); //~WARN: EnumerableSet
            } catch {
                return;
            }
        }
    }

    // The tried call's own arguments run before any clause is dispatched: a removal there is
    // followed by whichever clause runs, and a falling clause iterates on the shrunk set.
    function removeInsideTriedCall(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            try this.probeFlag(holders.remove(target)) {} catch {} //~WARN: EnumerableSet
        }
    }

    function probe() external {}

    function probeFlag(bool) external {}

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

    // A single storage-reference parameter is one set wherever the caller took it from:
    // walking and removing through it reports.
    function removeThroughSameStorageParameter(EnumerableSet.AddressSet storage set) internal {
        for (uint256 i = 0; i < set.length(); i++) {
            set.remove(set.at(i)); //~WARN: EnumerableSet
        }
    }

    // A documented limit: two storage-reference parameters are read as two distinct sets, so a
    // helper walking one and removing through the other goes unreported when a caller hands it
    // the same set twice. Judging that would take reading every call site, an interprocedural
    // pass this detector does not run; reading the parameters as aliased instead would report
    // every helper its callers use with two distinct sets.
    function walkOneRemoveOther(
        EnumerableSet.AddressSet storage walked,
        EnumerableSet.AddressSet storage pruned
    ) internal {
        for (uint256 i = 0; i < walked.length(); i++) {
            pruned.remove(walked.at(i));
        }
    }

    function helperHandedTheSameSetTwiceIsMissed() public {
        walkOneRemoveOther(holders, holders);
    }
}

// A user library attached to the EnumerableSet type, with its own `remove` overload.
library Helpers {
    function remove(EnumerableSet.AddressSet storage, uint256 index) internal pure {
        index += 1;
    }
}
