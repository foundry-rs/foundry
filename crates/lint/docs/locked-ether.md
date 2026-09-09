# Locked Ether

**Severity**: `Med`
**ID**: `locked-ether`

Flags contracts that can receive Ether (via `payable` functions, `receive()`, or a payable
`fallback()`) but expose no code path that can send Ether out. Any Ether sent to such a contract is
permanently trapped.

## What it does

For each concrete or abstract contract that has a payable entry point (`receive()`, payable
`fallback()`, payable constructor, or any payable function — directly or through inheritance),
the lint looks for an expression that can move Ether out:

- `addr.transfer(amount)` / `addr.send(amount)` with a non-zero amount.
- A call carrying a non-zero `{value: x}` option, such as `addr.call{value: x}(...)` or
  `new C{value: x}(...)`.
- `addr.delegatecall(...)` / `addr.callcode(...)`.
- `selfdestruct(addr)`.

If none is found, the contract is reported as locked at the contract's name.

## Why is this bad?

A contract that accepts Ether but cannot pay it back permanently traps user funds, with no way to
recover them. This is almost always a bug — typically a missing `withdraw()` function, a forgotten
access-controlled transfer, or a confused use of `payable` — and is hard to spot during review
because each individual function looks correct.

## Example

### Bad

```solidity
contract Vault {
    // Accepts ETH...
    receive() external payable {}

    // ...but provides no way to send it back out.
}
```

### Good

```solidity
contract Vault {
    address payable public immutable owner;

    constructor() {
        owner = payable(msg.sender);
    }

    receive() external payable {}

    function withdraw(uint256 amount) external {
        require(msg.sender == owner, "not owner");
        owner.transfer(amount);
    }
}
```
