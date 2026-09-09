# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

## What it does

Flags `EnumerableSet.remove` inside a loop that also reads the same set with `at`
using an increasing index.

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
