# Locked Ether

**Severity**: `Med`
**ID**: `locked-ether`

Flags contracts that can receive Ether (via `payable` functions, `receive()`, or a payable
`fallback()`) but expose no code path that can send Ether out. Any Ether sent to such a contract is
permanently trapped.

## What it does

For every concrete or abstract contract, the lint:

1. Checks whether the contract — or any contract in its inheritance chain — has at least one
   payable entry point (`receive()`, payable `fallback()`, payable constructor, or any payable
   function).
2. Walks every function defined in the contract and its linearized bases looking for a code path
   that can move Ether out. The following are recognized as ETH-sending operations:
   - `addr.transfer(amount)` and `addr.send(amount)` with a non-literal-zero amount.
   - Any call carrying a non-zero `{value: x}` option, including
     `addr.call{value: x}(...)` and `new C{value: x}(...)`.
   - Low-level `addr.delegatecall(...)` / `addr.callcode(...)` (the callee runs in this contract's
     context and can `selfdestruct`, draining the balance).
   - The `selfdestruct(addr)` builtin.
3. Internal and library calls whose callee statically resolves to a function are followed
   transitively, so a withdrawal helper buried behind several internal hops is detected. External
   calls through unresolved member access are not followed, to keep false positives down.

If no ETH-sending path is found, the lint reports the contract as locked at the contract's name.

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
