# Locked Ether

**Severity**: `Med`
**ID**: `locked-ether`

## What it does

For each concrete or abstract contract that has a payable entry point (`receive()`, payable
`fallback()`, payable constructor, or any payable function — directly or through inheritance),
the lint looks for an expression that can move Ether out:

- `addr.transfer(amount)` / `addr.send(amount)` with a non-zero amount.
- A call carrying a non-zero `{value: x}` option, such as `addr.call{value: x}(...)` or
  `new C{value: x}(...)`.
- `addr.delegatecall(...)` / `addr.callcode(...)`.
- `selfdestruct(addr)`.

If no such expression is found, the contract is reported. Finding one does not prove that
a withdrawal is reachable or authorized correctly.

## Why is this bad?

A contract that accepts Ether but cannot pay it back permanently traps user funds, with no way to
recover them. This is almost always a bug — typically a missing `withdraw()` function, a forgotten
access-controlled transfer, or a confused use of `payable` — and is hard to spot during review
because each individual function looks correct.

## Example

```solidity
contract Vault {
    // Accepts ETH...
    receive() external payable {}

    // ...but provides no way to send it back out.
}
```

Use instead:

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
