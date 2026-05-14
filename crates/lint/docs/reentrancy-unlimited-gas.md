# Reentrancy through unlimited-gas ETH calls

**Severity**: `High`
**ID**: `reentrancy-unlimited-gas`

Flags uncapped ETH-transferring low-level `call` operations when state read before the call is
written after the call.

## What it does

Warns when a function performs `.call{value: ...}(...)` without a concrete gas cap, or with
`gas: gasleft()`, and later writes a state variable that was read before the call on the same
reachable path. Local internal helper calls and modifiers are analyzed when their bodies are
available.

## Why is this bad?

Unlike `transfer` and `send`, low-level `call` forwards all remaining gas by default. A malicious
recipient can run complex fallback logic and re-enter the caller before later state changes occur.
If the function uses stale state read before the call and updates that state only afterward, the
recipient may be able to repeat or reorder effects.

This lint is intentionally conservative to avoid noisy findings: it does not report event-only
ordering issues, unrelated state writes, zero-value calls, constructor-time calls, or calls with an
explicit gas cap.

## Example

### Bad

```solidity
function withdraw() external {
    uint256 amount = balances[msg.sender];
    (bool ok, ) = payable(msg.sender).call{value: amount}("");
    require(ok, "transfer failed");
    balances[msg.sender] = 0;
}
```

### Good

```solidity
function withdraw() external {
    uint256 amount = balances[msg.sender];
    balances[msg.sender] = 0;
    (bool ok, ) = payable(msg.sender).call{value: amount}("");
    require(ok, "transfer failed");
}
```
