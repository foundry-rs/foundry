//@compile-flags: --only-lint enumerable-loop-removal
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {EnumerableSet as ES} from "./auxiliary/EnumerableSetLib.sol";

// Tests for `enumerable-loop-removal`: EnumerableSet removal is swap-and-pop, so calling
// `remove` inside a loop that also iterates the same set with `at` at an index the loop advances
// upward skips elements or reads out-of-bounds indices.
//
// The detector reports only the shape it can judge without a flow analysis: an unconditional
// ascending cadence (`i++`, `i += 1`, `i = i + 1` on the straight line of the body), an `at` read
// at that cadence, a `remove` on the same set, and a straight-line body. Calls resolve through the
// type checker, so the method-call and library-qualified forms are both covered, and storage-
// reference aliases are followed to the set they name where the loop runs.
//
// Anything outside that shape is left clean rather than guessed at: a conditional or mixed
// cadence, a composite index, a `break`/`continue`/`return`/`revert`, inline assembly, a nested
// loop, or a `try`. Expression-level short-circuits and ternaries remain structural expressions
// and are scanned. Some clean cases are genuine corruptions the detector deliberately does not
// report; they are grouped under "Documented limitations" below. Distinguishing them would take
// the flow or value analysis this detector does not run.

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

    error Stop();

    // ─────────────────────────────────────────────────────────────────────────────
    // Reported: unconditional ascending cadence, straight-line body, same set.
    // ─────────────────────────────────────────────────────────────────────────────

    function removeWhileIteratingFor() public {
        // `remove` shrinks the set while `at(i)` walks it upward: elements are skipped.
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
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

    function removeQualifiedForm() public {
        // The library-qualified form of the unsafe pattern: same set, ascending index.
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
        // An import alias renames the call site, not the declared library the calls resolve to.
        for (uint256 i = 0; i < ES.length(imported); i++) {
            ES.remove(imported, ES.at(imported, i)); //~WARN: EnumerableSet
        }
    }

    function removeUintSet() public {
        for (uint256 i = 0; i < ids.length(); i++) {
            ids.remove(ids.at(i)); //~WARN: EnumerableSet
        }
    }

    // A named literal index drains at a fixed position, so it is clean; a named varying index in
    // the method form still walks the loop.
    function removeMethodFormNamedVaryingIndex() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(holders.at({index: i})); //~WARN: EnumerableSet
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

    // A state cursor advanced by the loop on its straight line walks the set the same way a local
    // does, and the removal skips the elements swapped into the slots behind the cursor.
    function removeAtAdvancingStateCursor() public {
        while (cursor < holders.length()) {
            holders.remove(holders.at(cursor)); //~WARN: EnumerableSet
            cursor++;
        }
    }

    // A varying mapping key cannot be read, so it may name the iterated set.
    function removeAtVaryingMappingKey(uint256 key) public {
        for (uint256 i = 0; i < keyed[1].length(); i++) {
            keyed[key].remove(keyed[1].at(i)); //~WARN: EnumerableSet
        }
    }

    // The alias may stand on either side of the walk; the reference is followed to `holders`.
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

    // The straight-line rebinding stands where the loop runs: `alias_` names `holders` here.
    function removeThroughReassignedStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = others;
        alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // Rebound only under a condition before the loop, the reference has no one binding where the
    // loop reads it, so it may name the removed set.
    function removeThroughConditionallyReboundAlias(bool swap) public {
        EnumerableSet.AddressSet storage alias_ = others;
        if (swap) {
            alias_ = holders;
        }
        for (uint256 i = 0; i < alias_.length(); i++) {
            holders.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // A reference bound to another reference takes what that one named right then; the later
    // rebinding of `source` does not reach back, so `alias_` still walks `others`.
    function removeThroughAliasBoundToEarlierAlias() public {
        EnumerableSet.AddressSet storage source = others;
        EnumerableSet.AddressSet storage alias_ = source;
        source = holders;
        for (uint256 i = 0; i < alias_.length(); i++) {
            others.remove(alias_.at(i)); //~WARN: EnumerableSet
        }
    }

    // A tuple-destructured reference never got a binding the walk can read and carries no
    // initializer, so it may name the removed set.
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

    // A single storage-reference parameter is one set wherever the caller took it from.
    function removeThroughSameStorageParameter(EnumerableSet.AddressSet storage set) internal {
        for (uint256 i = 0; i < set.length(); i++) {
            set.remove(set.at(i)); //~WARN: EnumerableSet
        }
    }

    // The three ascending step forms all pace the loop: `i += 1` and `i = i + 1` read the same as
    // `i++`, whether in the `for`'s next-step or on the body's straight line.
    function removeWithCompoundStep() public {
        for (uint256 i = 0; i < holders.length(); i += 1) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeWithPlainAssignStep() public {
        for (uint256 i = 0; i < holders.length(); i = i + 1) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
        }
    }

    function removeWithInBodyStep() public {
        for (uint256 i = 0; i < holders.length();) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
            i++;
        }
    }

    // The step lands inside an `unchecked` block, transparent to the straight-line reading.
    function removeWithUncheckedStep() public {
        for (uint256 i = 0; i < holders.length();) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
            unchecked {
                ++i;
            }
        }
    }

    // A step of more than one still ascends and still walks into swapped-in tail elements.
    function removeWithStepOfTwo() public {
        for (uint256 i = 0; i < holders.length(); i += 2) {
            holders.remove(holders.at(i)); //~WARN: EnumerableSet
        }
    }

    // Expression-level conditions do not make the body structurally branch. The detector scans
    // both arms and may report a `remove` even when runtime values skip it.
    function removeInShortCircuitExpression(bool enabled, address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            bool removed = enabled && holders.remove(target); //~WARN: EnumerableSet
            removed;
        }
    }

    function removeInTernaryExpression(bool enabled, address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            bool removed = enabled ? holders.remove(target) : false; //~WARN: EnumerableSet
            removed;
        }
    }

    // The remove argument is not correlated with the `at` result. Removing the current tail does
    // not shift another value, but this still matches the structural same-set pattern.
    function removeTailWhileReadingAscending() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            holders.remove(holders.at(holders.length() - 1)); //~WARN: EnumerableSet
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Clean: no ascending cadence, no `at` on the removed set, or a different set.
    // ─────────────────────────────────────────────────────────────────────────────

    // Draining at the literal index 0 is safe: the swap refills the read position.
    function removeWhileIteratingWhile() public {
        while (holders.length() > 0) {
            address value = holders.at(0);
            holders.remove(value);
        }
    }

    // The `at` walks a different EnumerableSet instance.
    function removeWithAtOnOtherSet() public {
        for (uint256 i = 0; i < others.length(); i++) {
            holders.remove(others.at(i));
        }
    }

    // Iterate with `at`, collect, then remove in a second loop that has no `at`.
    function collectThenRemove() public {
        address[] memory toRemove = new address[](holders.length());
        for (uint256 i = 0; i < holders.length(); i++) {
            toRemove[i] = holders.at(i);
        }
        for (uint256 i = 0; i < toRemove.length; i++) {
            holders.remove(toRemove[i]);
        }
    }

    // No `remove` at all.
    function atOnlyInLoop() public view returns (uint256 count) {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) != address(0)) count++;
        }
    }

    function removeOutsideLoop(address value) public {
        holders.remove(value);
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

    // The qualified form on another set, written with named arguments.
    function removeNamedArgumentsOnOtherSet() public {
        for (uint256 i = 0; i < others.length(); i++) {
            EnumerableSet.remove({
                value: EnumerableSet.at({index: i, set: others}),
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

    uint256 internal constant FIRST = 0;

    // A literal, a cast of one, a named `constant`, or constant arithmetic names no ascending
    // cadence: the read drains position zero, which swap-and-pop keeps refilling.
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

    function removeAtConstantArithmeticIndex() public {
        while (holders.length() > 0) {
            holders.remove(holders.at(0 + 0));
        }
    }

    // A cursor the loop never advances is not an ascending cadence: the read drains the position
    // swap-and-pop keeps refilling, and the loop ends when the length comes down to it.
    function removeAtStationaryCursor() public {
        while (holders.length() > position) {
            holders.remove(holders.at(position));
        }
    }

    // This exact reverse drain removes the current tail, so swap-and-pop does not move another
    // value. Descending cadences are outside the reported shape regardless; an unsafe descending
    // case is pinned under the documented limitations below.
    function removeAtDescendingIndex() public {
        for (uint256 i = holders.length(); i > 0; i--) {
            holders.remove(holders.at(i - 1));
        }
    }

    function removeAtDescendingAssignedIndex() public {
        uint256 i = holders.length();
        while (i > 0) {
            i = i - 1;
            holders.remove(holders.at(i));
        }
    }

    // The reference is rebound only after the loop: where the loop runs it still names `others`.
    function removeThroughAliasReboundAfterTheLoop() public {
        EnumerableSet.AddressSet storage alias_ = others;
        for (uint256 i = 0; i < alias_.length(); i++) {
            holders.remove(alias_.at(i));
        }
        alias_ = holders;
    }

    // At the loop `alias_` is `holders`, so removing from `others` touches nothing it iterates.
    function removeAnotherSetThroughReassignedStorageAlias() public {
        EnumerableSet.AddressSet storage alias_ = others;
        alias_ = holders;
        for (uint256 i = 0; i < holders.length(); i++) {
            others.remove(alias_.at(i));
        }
    }

    // A copy of a cursor the loop never moves carries no cadence: still a fixed-slot drain.
    function removeAtCopyOfStationaryCursor() public {
        while (holders.length() > position) {
            uint256 idx = position;
            holders.remove(holders.at(idx));
        }
    }

    // A no-op step (`i += 0`, likewise `i = i`) does not advance, so it names no ascending
    // cadence: the loop drains the fixed slot swap-and-pop keeps refilling, which is safe.
    function removeWithNoOpStep() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i += 0;
        }
    }

    // Every write to a cadence must be a supported ascending step. These both leave `i` at zero,
    // so swap-and-pop refills the slot read on the next turn.
    function removeWithIncrementThenDecrement() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            i--;
        }
    }

    function removeWithIncrementThenReset() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            i = 0;
        }
    }

    // Mentioning the cadence inside an expression does not make it the paced index. Modulo one
    // always reads slot zero, which the swap refills while the set drains.
    function removeAtModuloCadence() public {
        uint256 i = 0;
        while (holders.length() > 0) {
            holders.remove(holders.at(i % 1));
            i++;
        }
    }

    // The mutation is rolled back and no next iteration is observable.
    function removeThenRevert() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            revert();
        }
    }

    function removeThenCustomErrorRevert() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            revert Stop();
        }
    }

    // Inline assembly is outside the straight-line analysis. In particular, a Yul revert rolls
    // back the removal and never reaches another iteration.
    function removeThenAssemblyRevert() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            assembly {
                revert(0, 0)
            }
        }
    }

    // A local declared in the body is initialized again on every turn. Its later increment does
    // not make it an ascending cadence across iterations.
    function removeWithRepeatedDeclarationReset() public {
        while (holders.length() > 0) {
            uint256 i = 0;
            holders.remove(holders.at(i));
            i++;
        }
    }

    function removeWithRepeatedTupleDeclarationReset() public {
        while (holders.length() > 0) {
            (uint256 i, uint256 other) = (0, 0);
            holders.remove(holders.at(i));
            i++;
            other;
        }
    }

    function removeWithDeleteReset() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            delete i;
        }
    }

    function removeWithTupleReset() public {
        uint256 i = 0;
        while (holders.length() > i) {
            holders.remove(holders.at(i));
            i++;
            (i, position) = (0, position);
        }
    }

    // Same `at`/`remove` names on a non-EnumerableSet type: out of scope.
    function customSetIsNotEnumerableSet() public {
        for (uint256 i = 0; i < bag.length(); i++) {
            if (bag.at(i) == address(0)) bag.remove(i);
        }
    }

    // A user library function named `remove` attached to the set type resolves to the helper, not
    // to EnumerableSet's swap-and-pop, so it is out of scope.
    function helperRemoveIsNotEnumerable() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == address(0)) holders.remove(i);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Documented limitations: real corruptions left unreported because judging them safely would
    // take flow or value analysis this detector does not run. It stays silent outside the narrow
    // supported shape.
    // ─────────────────────────────────────────────────────────────────────────────

    // A conditional removal whose set the loop keeps iterating: safe only if the mutating path
    // exits, which is not read here. Not reported: the body has an `if`.
    function removeUnderConditionKeepsIterating(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == target) holders.remove(target);
        }
    }

    // A `continue` comes round again on the shrunk set, but the body is not straight-line.
    function removeAndContinue(address target) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            if (holders.at(i) == target) {
                holders.remove(target);
                continue;
            }
        }
    }

    // A removal in a nested loop mutates the set the outer loop walks; nested loops are not read.
    function removeInInnerLoop() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            for (uint256 j = 0; j < 2; j++) {
                holders.remove(value);
            }
        }
    }

    // A removal decided by a short-circuit whose other operand can steer past the exit; the body
    // has a `break`, so it is not read.
    function removeConditionCarriesAnotherOperand(address target, bool flag) public {
        for (uint256 i = 0; i < holders.length(); i++) {
            pending = holders.at(i);
            if (holders.remove(target) && flag) break;
        }
    }

    // A copy of the ascending cadence walks the loop the same way, but copy propagation is not
    // run: `at(idx)` is not tied back to `i`.
    function removeAtCopiedIndex() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            uint256 idx = i;
            holders.remove(holders.at(idx));
        }
    }

    // Composite indices are outside the deliberately narrow `at(i)` shape, even when their value
    // equals the cadence and the loop has the same corruption.
    function removeAtCompositeCadenceIndex() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            holders.remove(holders.at(i + 0));
        }
    }

    // Unlike the safe reverse drain above, removing another value can move the already-read tail
    // into a slot this descending walk has not visited. All descending traversals are deliberately
    // unreported because the detector does not prove the relationship between `at` and `remove`.
    function removeAnotherValueWhileDescending(address target) public {
        for (uint256 i = holders.length(); i > 0; i--) {
            pending = holders.at(i - 1);
            holders.remove(target);
        }
    }

    // A descending walk that also steps up on some turns can revisit a swapped-in slot; the body
    // has an `if`, so direction is not analyzed.
    function removeAtDescendingIndexAlsoSteppedUp(bool skipTwo) public {
        for (uint256 i = holders.length(); i > 0; i--) {
            holders.remove(holders.at(i - 1));
            if (skipTwo) {
                i += 2;
            }
        }
    }

    // A member cursor advanced inside a nested loop paces the outer walk, but a member expression
    // is not a bare-identifier cadence and the nested loop is not read.
    function removeWithMemberCursor() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            for (uint256 k = 0; k < 1; k++) {
                pending = holders.at(cursor);
                cursor++;
            }
            holders.remove(pending);
        }
    }

    // A removal in a `try` clause that falls through iterates on the shrunk set; `try` is not read.
    function removeInTryClause() public {
        for (uint256 i = 0; i < holders.length(); i++) {
            address value = holders.at(i);
            try this.probe() {
                holders.remove(value);
            } catch {
                return;
            }
        }
    }

    function probe() external {}

    // A helper handed the same set twice: two distinct storage-reference parameters are read as
    // distinct sets, so this goes unreported. Reading them as aliased would warn on every helper a
    // caller uses with two different sets; judging it would take an interprocedural pass.
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
