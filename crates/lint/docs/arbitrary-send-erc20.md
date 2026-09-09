# Arbitrary ERC20 send

**Severity**: `High`
**ID**: `arbitrary-send-erc20`

`transferFrom` (and `safeTransferFrom`) move tokens from any address that has
previously approved this contract. If the `from` argument is taken from
user-controlled input without being constrained to `msg.sender` or
`address(this)`, an attacker can pull tokens from any wallet that has an
outstanding allowance to the vulnerable contract.

## What it does

Flags ERC20 `transferFrom` and `safeTransferFrom` calls whose `from` argument is not
constrained to `msg.sender` or `address(this)`, including SafeERC20 library calls.

## Related

A prior EIP-2612 `permit(owner, address(this), …)` does **not** suppress
the warning — the transfer is instead reported as
[`arbitrary-send-erc20-permit`](https://getfoundry.sh/forge/linting/arbitrary-send-erc20-permit), since
non-EIP-2612 tokens with a fallback can silently accept the permit and
let any prior allowance be drained.

## Why is this bad?

If a user has approved the contract to spend their tokens (e.g. for a swap or
deposit they expect to perform later), an attacker can call a function that
takes an arbitrary `from` and instruct the contract to transfer those tokens
to themselves. This is one of the most common ways funds are drained from
DeFi protocols.

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
