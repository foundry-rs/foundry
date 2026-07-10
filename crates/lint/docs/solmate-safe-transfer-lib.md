# Solmate SafeTransferLib

**Severity**: `Low`
**ID**: `solmate-safe-transfer-lib`

Flags token operations of solmate's `SafeTransferLib`, which does not check that the token has code in its released version.

## What it does

Reports a reference, called or used as a value, that resolves to `safeTransfer`, `safeTransferFrom` or `safeApprove` declared in a library named exactly `SafeTransferLib` whose source comes from a solmate package path (`lib/solmate`, `solmate/...`). Resolution goes through the type checker, so the `using for` method form, the library-qualified form and import aliases are all recognized, while same-name functions declared in other libraries (Uniswap's `TransferHelper` style) and same-name libraries from other packages (Solady's `SafeTransferLib` checks token code on the empty-return path) stay out of scope. `safeTransferETH` involves no token code and stays clean. A vendored solmate copy under a path that does not name solmate is not recognized.

Aderyn's detector of the same name flags the import directive whose path contains `solmate` and `SafeTransferLib`; resolving the calls instead anchors the warning where the risk sits and skips files that import the library without using it.

## Why is this bad?

In the released solmate v6, a token call that returns no data is treated as a success without checking that the token has code (`success := 1` on the empty-return path), unlike OpenZeppelin's `SafeERC20`. A token operation against an address with no code, a wrong address, a not-yet-deployed or a self-destructed token, is therefore a silent no-op that looks like a successful transfer. The unreleased solmate main branch has since added a code check to the empty-return path; on a released version, the mitigation is to verify the token has code, or to use OpenZeppelin's `SafeERC20`.

## Example

### Bad

```solidity
using SafeTransferLib for ERC20;

function pay(ERC20 token, address to, uint256 amount) internal {
    token.safeTransfer(to, amount);
}
```

### Good

```solidity
using SafeERC20 for IERC20;

function pay(IERC20 token, address to, uint256 amount) internal {
    token.safeTransfer(to, amount);
}
```

The lint flags every resolved solmate reference and does not recognize a manual code check
before the call. A call guarded by `require(address(token).code.length > 0, ...)` mitigates
the pitfall but still reports; suppress it explicitly:

```solidity
require(address(token).code.length > 0, "token has no code");
// forge-lint: disable-next-line(solmate-safe-transfer-lib)
token.safeTransfer(to, amount);
```
