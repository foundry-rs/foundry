# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates a set with `at`.

## What it does

Reports a call `set.remove(...)` where the receiver is a struct declared in a library named `EnumerableSet` (OpenZeppelin's `AddressSet`, `UintSet`, `Bytes32Set`), when an enclosing loop also contains a call `at(...)` on an EnumerableSet, not necessarily the same instance. This mirrors Aderyn's `enumerable-loop-removal` detector, with two differences: the `at` call must also be typed as an EnumerableSet (Aderyn matches any member named `at`), and any enclosing loop counts (Aderyn only checks the closest ancestor of each loop kind).

Only the combination is dangerous, so neither half fires alone:

- `remove` in a loop without `at` is the recommended collect-then-remove pattern;
- `at` in a loop without `remove` is a plain read;
- `remove` outside a loop is fine;
- the same method names on a type that is not an EnumerableSet are out of scope.

Calls reached indirectly through a function invoked from the loop are not analyzed.

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
