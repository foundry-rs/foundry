# Incorrect Strict Equality

**Severity**: `Med`
**ID**: `incorrect-strict-equality`

## What it does

Reports any strict-equality or strict-inequality expression (`==` or `!=`) whose left or right
operand contains:

- `<expr>.balance`, the ETH balance of an address, or
- `<expr>.balanceOf(<args>)`, an ERC-20 token balance call.

Comparisons involving arithmetic on a balance, such as `address(this).balance + 1 == target`,
are also flagged.

Exact `msg.value` checks, such as requiring a fixed payment, are not flagged by this lint.

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

Use instead:

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
