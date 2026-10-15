# No-ETH read-before-write reentrancy

**Severity**: `Med`
**ID**: `reentrancy-no-eth`

## What it does

Reports public or external functions that read a state variable, make an external call
without sending ETH, and then write the same state variable.

## Why is this bad?

Even without ETH transfer, an external call can invoke attacker-controlled code. If that code
re-enters before later state changes occur, the original function may continue with stale state and
overwrite or reuse values that changed during the reentrant execution.

ETH-transferring calls, view or pure interface calls, unrelated state writes, and constructor-time
calls are excluded. A custom reentrancy guard may still produce a warning; review its protection
before suppressing the lint.

## Example

```solidity
function claim(IHook hook) external {
    uint256 amount = balances[msg.sender];
    hook.notify(amount);
    balances[msg.sender] = 0;
}
```

Use instead:

```solidity
function claim(IHook hook) external {
    uint256 amount = balances[msg.sender];
    balances[msg.sender] = 0;
    hook.notify(amount);
}
```
