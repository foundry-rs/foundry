# No-ETH read-before-write reentrancy

**Severity**: `Med`
**ID**: `reentrancy-no-eth`

Flags external calls that do not transfer ETH when state read before the call is written after the
call on the same reachable path.

## What it does

Warns when a public or external entry point reads a state variable, performs a reentrant external
call that does not send ETH, and later writes the same state variable. Local internal helper calls
and modifiers are analyzed when their bodies are available. This uses Slither's
`reentrancy-no-eth` detector name.

## Why is this bad?

Even without ETH transfer, an external call can invoke attacker-controlled code. If that code
re-enters before later state changes occur, the original function may continue with stale state and
overwrite or reuse values that changed during the reentrant execution.

This lint is intentionally conservative to avoid noisy findings: it does not report ETH-transferring
calls, view or pure interface calls, unrelated state writes, or constructor-time calls. It does not
attempt to prove custom guard modifiers are effective.

## Example

### Bad

```solidity
function claim(IHook hook) external {
    uint256 amount = balances[msg.sender];
    hook.notify(amount);
    balances[msg.sender] = 0;
}
```

### Good

```solidity
function claim(IHook hook) external {
    uint256 amount = balances[msg.sender];
    balances[msg.sender] = 0;
    hook.notify(amount);
}
```
