# ETH reentrancy through uncapped calls

**Severity**: `High`
**ID**: `reentrancy-eth`

Flags uncapped ETH-transferring low-level `call` operations when state read before the call is
written after the call.

## What it does

Reports low-level `.call{value: ...}(...)` operations without a concrete gas cap, including
`gas: gasleft()`, when a state variable read before the call is written after it.

## Why is this bad?

Unlike `transfer` and `send`, low-level `call` forwards all remaining gas by default. A malicious
recipient can run complex fallback logic and re-enter the caller before later state changes occur.
If the function uses stale state read before the call and updates that state only afterward, the
recipient may be able to repeat or reorder effects.

Event-only ordering issues, unrelated state writes, zero-value calls, constructor-time calls,
and calls with a concrete gas cap are excluded. A gas cap alone is not a reentrancy defense.

## Example

```solidity
function withdraw() external {
    uint256 amount = balances[msg.sender];
    (bool ok, ) = payable(msg.sender).call{value: amount}("");
    require(ok, "transfer failed");
    balances[msg.sender] = 0;
}
```

Use instead:

```solidity
function withdraw() external {
    uint256 amount = balances[msg.sender];
    balances[msg.sender] = 0;
    (bool ok, ) = payable(msg.sender).call{value: amount}("");
    require(ok, "transfer failed");
}
```
