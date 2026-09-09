# Unchecked ERC20 transfer return value

**Severity**: `High`
**ID**: `erc20-unchecked-transfer`

## What it does

Warns when a function with the same signature as
`transfer(address,uint256)` or `transferFrom(address,address,uint256)` and a `bool` return type is
invoked but the result is not checked.

## Why is this bad?

The ERC20 spec allows tokens to signal failure by returning `false` instead of reverting. Ignoring
the return value lets a "failed" transfer go unnoticed, allowing accounting to drift and creating
common DeFi exploits. Use a wrapper such as OpenZeppelin's `SafeERC20` or check the boolean
explicitly.

## Example

```solidity
token.transfer(to, amount);
token.transferFrom(from, to, amount);
```

Use instead:

```solidity
require(token.transfer(to, amount), "transfer failed");
require(token.transferFrom(from, to, amount), "transferFrom failed");

// or use SafeERC20
SafeERC20.safeTransfer(token, to, amount);
```
