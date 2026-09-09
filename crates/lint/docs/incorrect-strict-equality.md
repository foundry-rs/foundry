# Incorrect Strict Equality

**Severity**: `Med`
**ID**: `incorrect-strict-equality`

Flags `==` and `!=` comparisons on values that can be manipulated by parties outside the
contract's control: ETH balances (`.balance`) and ERC-20 balances (`.balanceOf(...)`).

## What it does

Reports any strict-equality or strict-inequality expression (`==` or `!=`) whose left or right
operand contains:

- `<expr>.balance`, the ETH balance of an address, or
- `<expr>.balanceOf(<args>)`, an ERC-20 token balance call.

Operands are inspected recursively, so `address(this).balance + 1 == target` is also flagged.

## Why is this bad?

These values can be influenced by parties outside the contract's control:

- **ETH balance**: An attacker can force-send ETH to any address via `selfdestruct`, making an
  exact-equality check permanently unreachable. Locks that guard on
  `address(this).balance == target` can be bypassed or bricked.
- **ERC-20 balance**: Tokens can be transferred to a contract directly, without triggering any
  hook. A guard like `token.balanceOf(address(this)) == 0` can be violated by donating a single
  token wei.

In both cases, use `>=` or `<=` instead of `==` / `!=` to express the intended invariant
without being fragile to external manipulation, or rely on internal accounting.

## Example

### Bad

```solidity
// ETH balance, bricked by selfdestruct donation
function withdraw() external {
    require(address(this).balance == 100 ether, "wrong balance");
    payable(owner).transfer(address(this).balance);
}

// ERC-20 balance, bypassable with a 1-wei token transfer
function claimWhenEmpty() external {
    require(token.balanceOf(address(this)) == 0, "not empty");
    // ...
}
```

### Good

```solidity
// Use >= / <= to tolerate externally-added funds
function withdraw() external {
    require(address(this).balance >= 100 ether, "insufficient balance");
    payable(owner).transfer(address(this).balance);
}

function claimWhenEmpty() external {
    // Track deposits internally and compare against that instead
    require(internalBalance == 0, "not empty");
}
```

## Notes

`.balance` is only flagged when the receiver can be proven to be of type `address` (or
`address payable`). Recognized receivers include:

- `address(...)` casts and `payable(...)` expressions,
- variables declared as `address` / `address payable`,
- built-in members returning an address (`msg.sender`, `tx.origin`, `block.coinbase`),
- struct fields declared as `address`,
- elements of `address[]` arrays and values of `mapping(... => address)` mappings,
- functions returning a single `address` value.

Member accesses with the name `balance` on other types (e.g. user-defined struct fields named
`balance` whose type is not `address`) are intentionally ignored to avoid false positives.

`msg.value` is **not** covered by this lint. Exact payment validation
(`require(msg.value == price, ...)`) is a normal pattern and is left to the developer.

`block.timestamp` equality is handled by the separate `block-timestamp` lint.

Review each occurrence and prefer internal accounting over direct balance reads for critical
invariants.
