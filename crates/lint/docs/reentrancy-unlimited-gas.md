# Reentrancy through fixed-gas ETH transfers

**Severity**: `Info`
**ID**: `reentrancy-unlimited-gas`

Flags state changes and event emissions that occur after Solidity's built-in `transfer` or `send`
on the same reachable path. These operations forward a fixed gas stipend, but opcode repricing can
make new callback behavior affordable within that stipend.

## What it does

The lint tracks `transfer` and `send` calls on address-typed receivers through public and external
entry points, modifiers, and bare internal helper calls. It warns when a later reachable operation
writes contract state or emits an event. Both calls are tracked regardless of the transferred
amount because even a zero-value call executes recipient code.

Contract methods that merely reuse the names `transfer` or `send` are excluded by checking the
receiver type. Low-level calls, including calls with an explicit 2,300 gas cap, are outside this
detector's Slither-compatible scope. Constructors, local-variable writes, effects that occur only
before the call, and effects on mutually exclusive paths are not reported.

## Why is this bad?

The fixed stipend is not a stable reentrancy boundary. Changes to EVM opcode costs can alter which
fallback operations fit within it, so code that assumes `transfer` or `send` can never call back may
become unsafe after a network upgrade. A callback before a later state write can observe stale
state, while a callback before a later event can reorder logs consumed by off-chain systems.

Apply checks-effects-interactions: commit state and emit its corresponding event before interacting
with the recipient. Use a reentrancy guard when the interaction cannot safely be moved last.

## Example

### Bad

```solidity
function withdraw(address payable recipient, uint256 amount) external {
    recipient.transfer(amount);
    balances[recipient] -= amount;
    emit Withdrawal(recipient, amount);
}
```

### Good

```solidity
function withdraw(address payable recipient, uint256 amount) external {
    balances[recipient] -= amount;
    emit Withdrawal(recipient, amount);
    recipient.transfer(amount);
}
```
