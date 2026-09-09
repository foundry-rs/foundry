# Reentrancy Events

**Severity**: `Low`
**ID**: `reentrancy-events`

Flags events emitted after an external interaction. Emitting state-change events only after the external call returns can mislead off-chain consumers — including indexers, subgraphs, monitoring tools, and bridges — that rely on log ordering to reconstruct contract state.

## What it does

Reports events emitted after an external interaction, such as a state-changing contract
call, low-level `call` or `delegatecall`, ETH `send` or `transfer`, or contract creation.
Static calls and `view` or `pure` calls are excluded.

## Why is this bad?

Reentrancy and off-chain ordering both depend on event sequence:

- A reentrant callee can observe (or trigger another contract to observe) events in an order that no longer reflects the final state of the calling contract.
- Indexers, bridges, and monitoring tools that consume logs in emission order may apply state transitions incorrectly when events are not emitted alongside the writes they describe.

Emitting the event **before** the external call ensures the log is anchored to the local state change, regardless of what the callee does.

## Example

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

Use instead:

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
