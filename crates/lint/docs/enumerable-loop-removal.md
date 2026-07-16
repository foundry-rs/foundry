# Enumerable loop removal

**Severity**: `High`
**ID**: `enumerable-loop-removal`

Flags `remove` on an EnumerableSet inside a loop that also iterates the same set with `at`.

## What it does

Reports a call to `EnumerableSet.remove` inside a loop that reads the same set with `EnumerableSet.at` at the loop's own ascending index. Calls are resolved through the type checker, so both the `using for` method form (`set.remove(...)`) and the library-qualified form (`EnumerableSet.remove(set, ...)`) are recognized, named arguments included. The library is identified only by being a library named exactly `EnumerableSet`; its origin and implementation are not verified, so a same-named user library can match while a renamed fork does not.

The report requires a shape that can be judged without a flow analysis:

- an **unconditional ascending cadence**: a bare loop index stepped upward on the straight line of the body, written `i++`, `i += 1`, `i = i + 1`, or `i = 1 + i` (a step of more than one and an `unchecked` step count too). A `for`'s next-step, a `while`'s in-body counter, and a `do-while`'s all qualify. Every write to that index must be one of these ascending forms; a decrement, reset, or unrecognized write leaves the loop unreported;
- an **`at` read at that cadence**: `set.at(i)` for the cadence variable `i`;
- a **`remove` on the same set**. Calls inside short-circuit and ternary expressions count when their arm may execute; an arm that a literal boolean condition proves unreachable is skipped;
- a **straight-line statement body**: no `if`, `try`, `break`, `continue`, `return`, `revert`, inline assembly, or nested loop.

Two set operands name the same set when they name the same storage path: a base variable followed by struct fields and literal mapping keys. A local `storage` reference is another name for what its last straight-line binding gave it, resolved where the loop runs, so `set = holders` iterates `holders`, and a reference rebound only after the loop still names what it did inside it. An operand the analysis cannot read (a varying mapping key, a call result, a reference rebound under a condition) is matched against every set, reporting a possibly-safe removal rather than missing an unsafe one.

These stay clean:

- `remove` in a loop without `at` is the recommended collect-then-remove pattern;
- `at` in a loop without `remove` is a plain read;
- `remove` outside a loop is fine;
- `at` on a different set instance, including two struct fields or two literal mapping keys;
- `at` at an index the loop never advances (`remove(at(0))`, a `constant`, a stationary cursor, a no-op step like `i += 0`): the swap refills the read position;
- the same function names outside the `EnumerableSet` library.

### Deliberately unreported

The detector omits statement shapes that need control-flow or value reasoning to decide whether removal corrupts iteration. This leaves some genuine corruptions unreported rather than guessing about their safe lookalikes:

- a removal under statement-level control (`if (cond) set.remove(set.at(i))` under an unconditional `i++`). A call inside a short-circuit or ternary expression is still scanned and can report unless a literal boolean proves that expression arm unreachable;
- a removal followed by `continue`, which comes round on the shrunk set;
- a removal in a nested loop, or in a `try` clause that falls through;
- a removal inside a statement-level exit condition whose short-circuit operand can steer past that exit;
- an index that is a straight-line copy of the cadence (`idx = i; set.at(idx)`) or composite arithmetic on it;
- a member or indexed cursor, or a cursor paced only inside a nested loop;
- any descending traversal. Removing the current tail while walking backward is safe, but removing another value can move an already-read tail into an unvisited slot; the detector proves neither relationship and reports neither case;
- a helper iterating one storage-reference parameter and removing through another, handed the same set twice.

This intentionally trades recall for a narrow, predictable reporting shape. The canonical ascending `remove(set.at(i))` loop is covered, while real corruptions outside that shape are missed. Within the supported shape, unresolved set operands are treated as possible aliases, so the detector can also warn when two operands are distinct at runtime.

The argument passed to `remove` is not related back to the value returned by `at`. Consequently, the lint can report a safe same-set removal, such as removing the current tail while separately reading the set at the ascending cadence, because tail removal does not shift another element into the walked prefix.

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
