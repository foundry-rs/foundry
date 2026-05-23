# Reentrancy Events

**Severity**: `Low`
**ID**: `reentrancy-events`

Flags `emit` statements that appear after an external call within the same function (or any internal helper it transitively calls). Emitting state-change events only after the external call returns can mislead off-chain consumers — including indexers, subgraphs, monitoring tools, and bridges — that rely on log ordering to reconstruct contract state.

## What it does

For every function body, the lint performs a control-flow analysis that tracks whether an external call has occurred on the path leading to each statement. External calls include:

- Low-level calls: `address.call(...)`, `delegatecall(...)`, `staticcall(...)` (with or without `{value: ...}` / `{gas: ...}` options).
- ETH sends: `address.transfer(...)`, `address.send(...)`.
- `this.method(...)` self-external calls.
- High-level external calls on interface or contract types (e.g. `IERC20(token).transfer(...)`).

External calls reached through internal/private/public helper functions and modifiers are also tracked transitively. When the analysis encounters an `emit` statement reachable from a path that already executed an external call, the statement is flagged.

## Why is this bad?

Reentrancy and off-chain ordering both depend on event sequence:

- A reentrant callee can observe (or trigger another contract to observe) events in an order that no longer reflects the final state of the calling contract.
- Indexers, bridges, and monitoring tools that consume logs in emission order may apply state transitions incorrectly when events are not emitted alongside the writes they describe.

Emitting the event **before** the external call ensures the log is anchored to the local state change, regardless of what the callee does.

## Example

### Bad

```solidity
contract BadCounter {
    uint256 public counter;
    event Counter(uint256 value);

    function count(IExternal d) external {
        counter += 1;
        d.notify();             // external call first ...
        emit Counter(counter);  // ... then the event (may be reordered by reentrancy)
    }
}
```

### Good

```solidity
contract GoodCounter {
    uint256 public counter;
    event Counter(uint256 value);

    function count(IExternal d) external {
        counter += 1;
        emit Counter(counter);  // emit event right after the state change
        d.notify();             // then perform the external call
    }
}
```
