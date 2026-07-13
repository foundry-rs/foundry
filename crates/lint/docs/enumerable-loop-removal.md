# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates the same set with `at`.

## What it does

Reports a call to `EnumerableSet.remove` when a loop reads the same set with `EnumerableSet.at` at a varying index and comes round again after the removal. Calls are resolved through the type checker, so both the `using for` method form (`set.remove(...)`) and the library-qualified form (`EnumerableSet.remove(set, ...)`) are recognized, named arguments included, and same-name functions from other libraries are not.

This is Aderyn's `enumerable-loop-removal` detector, narrower on what earns a report and wider on where the two calls may sit. Aderyn captures a `remove` member access whose type string contains `EnumerableSet` as soon as a member access named `at` appears under the closest ancestor of each loop kind. Here the calls must resolve to functions of a library named `EnumerableSet`, the `at` and the `remove` must operate on the same set, the index must vary, and the loop must be able to come round after the removal. In exchange every enclosing loop counts, not only the closest one of each kind.

An `at` read walks a loop only when its index names that loop's own moving cadence: `holders.at(i)` walks the `i` loop wherever the read sits, even inside a nested one. A loop's cadence is a variable it advances as it turns, from its old value (a `for`'s `i++`, a `while`'s `i += n` or `i = i + 1`), reading it either in the loop's own body outside any nested loop or in one of its conditions, its guard or an `if (...) break`. An index that names no cadence reads a slot the loop does not advance over, so it reports nothing: a literal, a `constant`, a stationary cursor the loop never moves, or a nested loop's own cursor, which stays that loop's even when declared outside the enclosing body, a function parameter or a hoisted local included; a cursor reset each turn (`j = 0`) does not progress and is no cadence. A cadence only ever stepped downward (`i--`, `i -= n`, `i = i - n`) is left out too: swap-and-pop moves the tail into the slot being emptied, and a descending walk never returns to a slot at or above the one it just read, so the reverse drain `for (uint256 i = set.length(); i > 0; i--) set.remove(set.at(i - 1))` skips nothing; one upward step anywhere makes the direction unknown and keeps the variable.

A removal anywhere under a loop mutates the set while that loop iterates, unless the loop is left before it comes round: a `break`, a `return`, or a revert ends it, while a `continue` and a `break` inside a nested loop do not. Each clause of a `try` runs on its own path, so a success clause that removes and returns is done with the loop even when a `catch` falls through empty. A removal that is itself the whole condition of an exiting branch is read the same way: `remove` answers true exactly when it took the value out, so `if (set.remove(value)) break` shrinks the set only on the path that leaves; another operand riding along (`remove(v) && flag`) can steer execution past the exit after the mutation and still reports.

Two set operands name the same set when they name the same storage path: a base variable followed by struct fields and literal mapping keys. A local `storage` reference is another name for what its last straight-line binding gave it, resolved where the loop runs: `set = holders` iterates `holders`, a reference rebound only after the loop still names what it did inside it, and a binding taken from another reference reads what that one named right then. An operand the analysis cannot read is matched against every set, which reports a safe removal rather than miss an unsafe one: a varying mapping key, a call result, or a reference rebound under a condition, inside the loop, or in any shape the walk cannot follow.

Only the combination on the same set is dangerous, so these stay clean:

- `remove` in a loop without `at` is the recommended collect-then-remove pattern;
- `at` in a loop without `remove` is a plain read;
- `remove` outside a loop is fine;
- `at` on a different set instance cannot be corrupted by the removal, two struct fields and two literal mapping keys naming two instances;
- `at` at an index the loop never moves is a drain (`remove(at(0))` style): the swap refills the read position, which holds for a number literal, a cast of one such as `uint256(0)`, a named `constant`, constant arithmetic like `0 + 0`, and a stationary cursor (`while (set.length() > position) set.remove(set.at(position))`);
- `at` at a cadence only stepped downward drains from the top, the reverse-loop pattern;
- a removal the loop never returns from, the find-and-remove-once pattern, `if (set.remove(value)) break` included;
- the same function names outside the `EnumerableSet` library are out of scope.

An index that mentions a nested loop's variable, such as `at(i + j)`, is read as that loop's own and does not report the enclosing one. A straight-line copy the loop takes of its cadence carries it: `uint256 idx = i;` walks the loop through `at(idx)` as `at(i)` does, and so does an index derived from the cadence, even when its arithmetic happens to walk downward, a conservative report. Calls reached indirectly through a function invoked from the loop are not analyzed.

The descending exemption reads direction syntactically and assumes the loop removes what it just read: an `unchecked` step that wraps around, a second removal per turn, or removing a different value on the way down (which can move the just-read tail into an unvisited slot and revisit it) are not modeled. Symmetrically, the removal deciding an exiting branch guarantees no further iteration, not no further read: a branch that reads `at` again after the removal and before leaving reads a shifted slot once.

The in-place filter that only advances its cursor when nothing was removed, `if (matches) { set.remove(value); } else { i++; }`, is correct under swap-and-pop, the swapped-in element being read on the next turn, but it still reports: telling it apart needs a path-sensitive reading of the cadence. Collect-then-remove or the descending drain express the same intent without the report.

Two shapes stay out of reach of a per-function pass. A counter stepped only inside a nested loop while an enclosing loop reads `at` it, whether the enclosing condition hides it behind an opaque predicate (`while (hasMore())`) or never names it at all: the counter then names neither the enclosing loop's own progression nor any of its conditions, so the read at it is not tied back to that loop and the removal goes unreported. And a helper that iterates one storage-reference parameter while removing through another, handed the same set twice at a call site: the two parameters are read as two distinct sets, since judging the calls would take reading every call site, and reading the parameters as aliased instead would report every helper its callers use with two distinct sets.

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
