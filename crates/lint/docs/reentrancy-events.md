# Reentrancy Events

**Severity**: `Low`
**ID**: `reentrancy-events`

## What it does

Reports events emitted after an external interaction, such as a state-changing contract
call, low-level `call` or `delegatecall`, ETH `send` or `transfer`, or contract creation.
Static calls and `view` or `pure` calls are excluded.

Calls in ordinary internal helpers and modifiers are followed, but interactions hidden in
library-qualified or `using for` internal calls may be missed.

## Why is this bad?

Reentrancy and off-chain ordering both depend on event sequence:

- A reentrant call can cause nested state changes and their events to be interleaved with the
  original operation, making log order differ from the order of the state changes it describes.
- Indexers, bridges, and monitoring tools that consume logs in emission order may apply state transitions incorrectly when events are not emitted alongside the writes they describe.

Emit the event alongside the state change it describes, before yielding control externally.
Contracts cannot read transaction logs during execution; this warning concerns off-chain consumers.

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
