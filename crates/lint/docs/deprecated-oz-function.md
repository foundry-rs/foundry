# Deprecated OpenZeppelin function

**Severity**: `Low`
**ID**: `deprecated-oz-function`

Flags references to OpenZeppelin functions the library has deprecated: `SafeERC20.safeApprove` and `AccessControl._setupRole`.

## What it does

Reports a reference, called or used as a value, that resolves to a function named `safeApprove` declared in a library named `SafeERC20` / `SafeERC20Upgradeable`, or to a function named `_setupRole` declared in a contract named `AccessControl` / `AccessControlUpgradeable`. Resolution goes through the type checker, so the `using for` method form, the library-qualified form, import aliases and inheritance through extensions are all recognized, while same-name functions declared elsewhere stay out of scope. The declaration must also come from an OpenZeppelin package path (`lib/openzeppelin-contracts`, `@openzeppelin/...`), so a local library or contract reusing the canonical name is not reported; the flip side is that a vendored OpenZeppelin copy under a path that does not name OpenZeppelin is not recognized. This mirrors Aderyn's `deprecated-oz-function` detector, which matches any identifier or member with those names in files importing an OpenZeppelin path.

## Why is this bad?

OpenZeppelin deprecated both functions in the 4.x line and removed them in 5.0, so they are dead ends for upgrades:

- `safeApprove` reverts when changing a non-zero allowance to another non-zero value; `safeIncreaseAllowance` / `safeDecreaseAllowance` are the replacements its deprecation note documents, and `forceApprove` (added in 4.9 for tokens behaving like USDT) sets an exact allowance safely.
- `_setupRole` was only intended for constructor setup and bypasses the role-admin checks; `_grantRole` is the supported replacement.

## Example

### Bad

```solidity
using SafeERC20 for IERC20;

function approveSpender(IERC20 token, address spender, uint256 amount) internal {
    token.safeApprove(spender, amount);
}

constructor(address admin) {
    _setupRole(DEFAULT_ADMIN_ROLE, admin);
}
```

### Good

```solidity
using SafeERC20 for IERC20;

function approveSpender(IERC20 token, address spender, uint256 amount) internal {
    token.forceApprove(spender, amount);
}

constructor(address admin) {
    _grantRole(DEFAULT_ADMIN_ROLE, admin);
}
```
