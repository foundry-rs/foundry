# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates the same set with `at`.

## What it does

Flags `EnumerableSet.remove` inside a loop that also reads the same set with `at`
using an increasing index.

Collecting elements in one loop and removing them in a separate loop avoids the warning
and the iteration hazard.

Complex loops may go unreported, and a warning does not prove that a particular removal
skips an element. Review which element is moved into the removed position and which
position the loop visits next.

## Why is this bad?

`EnumerableSet.remove` is swap-and-pop: it moves the last element into the removed slot and shrinks the set. Iterating by an ascending index with `at` while removing skips the swapped-in elements or reads out-of-bounds indices, so some elements are silently never visited.

## Example

```solidity
for (uint256 i = 0; i < set.length(); i++) {
    set.remove(set.at(i));
}
```

Use instead:

```solidity
// Remove selectively: collect during the loop, remove after it.
address[] memory toRemove = new address[](set.length());
uint256 count = 0;
for (uint256 i = 0; i < set.length(); i++) {
    address value = set.at(i);
    if (shouldRemove(value)) toRemove[count++] = value;
}
for (uint256 i = 0; i < count; i++) {
    set.remove(toRemove[i]);
}
```
