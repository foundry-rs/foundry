# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates the same set with `at`.

## What it does

Reports a call to `EnumerableSet.remove` inside a loop that reads the same set with `EnumerableSet.at` at the loop's own ascending index. Calls are resolved through the type checker, so both the `using for` method form (`set.remove(...)`) and the library-qualified form (`EnumerableSet.remove(set, ...)`) are recognized, named arguments included, and same-name functions from other libraries are not.

The report requires a shape that can be judged without a flow analysis:

- an **unconditional ascending cadence**: a bare loop index stepped upward on the straight line of the body, written `i++`, `i += 1`, or `i = i + 1` (a step of more than one and an `unchecked` step count too). A `for`'s next-step, a `while`'s in-body counter, and a `do-while`'s all qualify;
- an **`at` read at that cadence**: `set.at(i)` for the cadence variable `i`;
- a **`remove` on the same set**, conditional or not;
- a **straight-line body**: no `if`, `try`, `break`, `continue`, `return`, or nested loop.

Two set operands name the same set when they name the same storage path: a base variable followed by struct fields and literal mapping keys. A local `storage` reference is another name for what its last straight-line binding gave it, resolved where the loop runs, so `set = holders` iterates `holders`, and a reference rebound only after the loop still names what it did inside it. An operand the analysis cannot read (a varying mapping key, a call result, a reference rebound under a condition) is matched against every set, reporting a possibly-safe removal rather than missing an unsafe one.

These stay clean:

- `remove` in a loop without `at` is the recommended collect-then-remove pattern;
- `at` in a loop without `remove` is a plain read;
- `remove` outside a loop is fine;
- `at` on a different set instance, including two struct fields or two literal mapping keys;
- `at` at an index the loop never advances (`remove(at(0))`, a `constant`, a stationary cursor, a no-op step like `i += 0`): the swap refills the read position;
- `at` at a cadence stepped downward (`for (i = set.length(); i > 0; i--) set.remove(set.at(i - 1))`): swap-and-pop moves the tail into the emptied slot, which a descending walk never revisits;
- the same function names outside the `EnumerableSet` library.

### Deliberately unreported

The detector prefers silence to a guess whenever a control-flow construct could change whether the removal corrupts the iteration. The following are genuine corruptions it does **not** report, because distinguishing them from their safe lookalikes needs a flow analysis this pass does not run:

- a conditional removal whose set the loop keeps iterating (`if (cond) set.remove(set.at(i))` under an unconditional `i++`);
- a removal followed by `continue`, which comes round on the shrunk set;
- a removal in a nested loop, or in a `try` clause that falls through;
- a removal decided by a short-circuit whose other operand can steer past an exit;
- an index that is a straight-line copy of the cadence (`idx = i; set.at(idx)`) or composite arithmetic on it;
- a member or state cursor, or a cursor paced only inside a nested loop;
- a descending walk that also steps up on some turns, which can revisit a swapped-in slot;
- a helper iterating one storage-reference parameter and removing through another, handed the same set twice.

This trades recall for precision: the canonical corruption is always reported, and no safe pattern is ever warned on.

## Why is this bad?

`EnumerableSet.remove` is swap-and-pop: it moves the last element into the removed slot and shrinks the set. Iterating by an ascending index with `at` while removing skips the swapped-in elements or reads out-of-bounds indices, so some elements are silently never visited.

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
