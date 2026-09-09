# Arbitrary ERC20 send

**Severity**: `High`
**ID**: `arbitrary-send-erc20`

## What it does

Flags ERC20 `transferFrom` and `safeTransferFrom` calls whose `from` argument is not
constrained to `msg.sender` or `address(this)`, including SafeERC20 library calls.

## Why is this bad?

If a user has approved the contract to spend their tokens (e.g. for a swap or
deposit they expect to perform later), an attacker can call a function that
takes an arbitrary `from` and instruct the contract to transfer those tokens
to themselves.

## Example

```solidity
function pull(address from, address to, uint256 amount) external {
    token.transferFrom(from, to, amount); // attacker may pass any `from`
}
```

Use instead:

```solidity
function deposit(uint256 amount) external {
    token.transferFrom(msg.sender, address(this), amount);
}

function pull(address from, address to, uint256 amount) external {
    require(from == msg.sender, "unauthorized");
    token.transferFrom(from, to, amount);
}
```
