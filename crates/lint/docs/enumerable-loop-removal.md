# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates the same set with `at`.

## What it does

Reports a call to `EnumerableSet.remove` when a loop reads the same set with `EnumerableSet.at` at a varying index and comes round again after the removal. Calls are resolved through the type checker, so both the `using for` method form (`set.remove(...)`) and the library-qualified form (`EnumerableSet.remove(set, ...)`) are recognized, named arguments included, and same-name functions from other libraries are not.

This is Aderyn's `enumerable-loop-removal` detector, narrower on what earns a report and wider on where the two calls may sit. Aderyn captures a `remove` member access whose type string contains `EnumerableSet` as soon as a member access named `at` appears under the closest ancestor of each loop kind. Here the calls must resolve to functions of a library named `EnumerableSet`, the `at` and the `remove` must operate on the same set, the index must vary, and the loop must be able to come round after the removal. In exchange every enclosing loop counts, not only the closest one of each kind.

An `at` read belongs to the loop whose cadence its index paces, not to the loop it is written in: `holders.at(i)` walks the `i` loop even from inside a nested one. A loop's cadence is a variable it advances as it turns, from its old value (a `for`'s `i++`, a `while`'s `i += n` or `i = i + 1`), reading it either in the loop's own body outside any nested loop or in one of its conditions, its guard or an `if (...) break`. A nested loop's own cursor, advanced and tested inside that loop, stays the nested loop's even when declared outside the enclosing body, a function parameter or a hoisted local included; a cursor reset each turn (`j = 0`) does not progress and is no cadence. A removal anywhere under a loop mutates the set while that loop iterates, unless the loop is left before it comes round: a `break`, a `return`, a revert, or a `try` whose every clause leaves ends it, while a `continue` and a `break` inside a nested loop do not.

Two set operands name the same set when they name the same storage path: a base variable followed by struct fields and literal mapping keys. A local `storage` reference is another name for what it was bound to, so `set = holders` iterates `holders`. An operand the analysis cannot read is matched against every set, which reports a safe removal rather than miss an unsafe one: a varying mapping key, a call result, or a `storage` reference the function binds again.

Only the combination on the same set is dangerous, so these stay clean:

- `remove` in a loop without `at` is the recommended collect-then-remove pattern;
- `at` in a loop without `remove` is a plain read;
- `remove` outside a loop is fine;
- `at` on a different set instance cannot be corrupted by the removal, two struct fields and two literal mapping keys naming two instances;
- `at` with an index fixed at compile time is a drain (`remove(at(0))` style): the swap refills the read position, which holds for a number literal, a cast of one such as `uint256(0)`, and a named `constant`; constant arithmetic like `0 + 0` is not folded and is treated as varying, a conservative report;
- a removal the loop never returns from, the find-and-remove-once pattern;
- the same function names outside the `EnumerableSet` library are out of scope.

An index that mentions a nested loop's variable, such as `at(i + j)`, is read as that loop's own and does not report the enclosing one. Calls reached indirectly through a function invoked from the loop are not analyzed.

One shape stays out of reach, since identifying a loop's cadence is syntactic: a loop whose condition is an opaque predicate (`while (hasMore())`) that hides the counter it advances, when that counter is stepped only inside a nested loop that iterates on something else. The counter then names neither the loop's own progression nor any condition, so the read at it is not tied back to the enclosing loop and the removal goes unreported, the way a value settled behind a function call is beyond a per-function pass.

## Why is this bad?

`EnumerableSet.remove` is swap-and-pop: it moves the last element into the removed slot and shrinks the set. Iterating by index with `at` while removing skips the swapped-in elements or reads out-of-bounds indices, so some elements are silently never visited.

## Example

### Bad

```solidity
for (uint256 i = 0; i < set.length(); i++) {
    set.remove(set.at(i));
}
```

### Good

```solidity
// removing everything: EnumerableSet has a dedicated function for it,
// much cheaper than removing one element at a time
set.clear();

// removing selectively: collect during the loop, remove after it
address[] memory toRemove = new address[](set.length());
for (uint256 i = 0; i < set.length(); i++) {
    if (shouldRemove(set.at(i))) toRemove[i] = set.at(i);
}
for (uint256 i = 0; i < toRemove.length; i++) {
    set.remove(toRemove[i]);
}
```
