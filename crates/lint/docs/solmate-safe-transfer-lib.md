# Solmate `SafeTransferLib`

**Severity**: `Low`
**ID**: `solmate-safe-transfer-lib`

## What it does

Reports uses of `safeTransfer`, `safeTransferFrom`, and `safeApprove` from solmate's
`SafeTransferLib`. ETH transfers and similarly named libraries from other packages are
excluded.

The check recognizes Solmate package paths, not the installed implementation's version.
A vendored copy under another package name can be missed, while a patched copy may still warn.

## Why is this bad?

Solmate v6 treats a token call that returns no data as successful without checking whether
the token address has code. A call to an address with no code can therefore look like a
successful transfer. Verify that the token has code or use OpenZeppelin's `SafeERC20`.

## Example

```solidity
using SafeTransferLib for ERC20;

function pay(ERC20 token, address to, uint256 amount) internal {
    token.safeTransfer(to, amount);
}
```

Use instead:

```solidity
using SafeERC20 for IERC20;

function pay(IERC20 token, address to, uint256 amount) internal {
    token.safeTransfer(to, amount);
}
```

A call guarded by `require(address(token).code.length > 0, ...)` mitigates this pitfall but
still produces a warning. After reviewing the guard, suppress the lint locally:

```solidity
require(address(token).code.length > 0, "token has no code");
// forge-lint: disable-next-line(solmate-safe-transfer-lib)
token.safeTransfer(to, amount);
```
