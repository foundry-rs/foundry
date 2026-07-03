# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates the same set with `at`.

## What it does

Reports a call to `EnumerableSet.remove` when an enclosing loop also reads the same set with `EnumerableSet.at` at a varying index. Calls are resolved through the type checker, so both the `using for` method form (`set.remove(...)`) and the library-qualified form (`EnumerableSet.remove(set, ...)`) are recognized, and same-name functions from other libraries are not. This mirrors Aderyn's `enumerable-loop-removal` detector, with two differences: the calls must resolve to functions of a library named `EnumerableSet` (Aderyn matches any members named `at` and `remove`), and any enclosing loop counts (Aderyn only checks the closest ancestor of each loop kind).

Only the combination on the same set is dangerous, so these stay clean:

- `remove` in a loop without `at` is the recommended collect-then-remove pattern;
- `at` in a loop without `remove` is a plain read;
- `remove` outside a loop is fine;
- `at` on a different set instance cannot be corrupted by the removal;
- `at` with a literal index is a drain (`remove(at(0))` style): the swap refills the read position;
- the same function names outside the `EnumerableSet` library are out of scope.

Two sets are told apart by the variable they are stored in; an operand too complex to name a single variable (a mapping entry, a call result) is conservatively treated as possibly the removed set. Calls reached indirectly through a function invoked from the loop are not analyzed.

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
