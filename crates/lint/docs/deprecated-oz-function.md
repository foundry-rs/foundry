# Deprecated OpenZeppelin function

**Severity**: `Low`
**ID**: `deprecated-oz-function`

## What it does

Reports uses of OpenZeppelin's `SafeERC20.safeApprove` and `AccessControl._setupRole`,
including their upgradeable variants.

## Why is this bad?

OpenZeppelin deprecated both functions in the 4.x line and removed them in 5.0, so they are dead ends for upgrades:

- `safeApprove` reverts when changing a non-zero allowance to another non-zero value; `safeIncreaseAllowance` / `safeDecreaseAllowance` are the replacements its deprecation note documents, and `forceApprove` (added in 4.9 for tokens behaving like USDT) sets an exact allowance safely.
- `_setupRole` was only intended for constructor setup and bypasses the role-admin checks; `_grantRole` is the supported replacement.

## Example

```solidity
using SafeERC20 for IERC20;

function approveSpender(IERC20 token, address spender, uint256 amount) internal {
    token.safeApprove(spender, amount);
}

constructor(address admin) {
    _setupRole(DEFAULT_ADMIN_ROLE, admin);
}
```

Use instead:

```solidity
using SafeERC20 for IERC20;

function approveSpender(IERC20 token, address spender, uint256 amount) internal {
    token.forceApprove(spender, amount);
}

constructor(address admin) {
    _grantRole(DEFAULT_ADMIN_ROLE, admin);
}
```
