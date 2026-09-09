# Solmate `SafeTransferLib`

**Severity**: `Low`
**ID**: `solmate-safe-transfer-lib`

Flags token operations of solmate's `SafeTransferLib`, which does not check that the token has code in its released version.

## What it does

Reports uses of `safeTransfer`, `safeTransferFrom`, and `safeApprove` from solmate's
`SafeTransferLib`. ETH transfers and similarly named libraries from other packages are
excluded.

## Why is this bad?

In the released solmate v6, a token call that returns no data is treated as a success without checking that the token has code (`success := 1` on the empty-return path), unlike OpenZeppelin's `SafeERC20`. A token operation against an address with no code, a wrong address, a not-yet-deployed or a self-destructed token, is therefore a silent no-op that looks like a successful transfer. The unreleased solmate main branch has since added a code check to the empty-return path; on a released version, the mitigation is to verify the token has code, or to use OpenZeppelin's `SafeERC20`.

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
