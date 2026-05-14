# Return bomb

**Severity**: `Low`
**ID**: `return-bomb`

Flags low-level external calls that set an explicit gas limit while copying unbounded returndata
into a `bytes` value.

## What it does

Detects low-level `call`, `delegatecall`, and `staticcall` expressions that specify `{gas: ...}`
and bind the raw returndata to `bytes memory` or an existing `bytes` variable.

## Why is this bad?

The gas option limits gas forwarded to the callee, but copying returndata into memory is paid by
the caller after the call returns. A malicious callee can return or revert with large returndata,
causing the caller to run out of gas while implicitly copying the result.

## Example

### Bad

```solidity
function callTarget(address target, bytes memory payload, uint256 gasLimit) external {
    (bool ok, bytes memory result) = target.call{gas: gasLimit}(payload);
    require(ok);
}
```

### Good

```solidity
function callTarget(address target, bytes memory payload, uint256 gasLimit) external {
    (bool ok, ) = target.call{gas: gasLimit}(payload);
    require(ok);
}
```

If the returndata is needed, copy only a bounded number of bytes with a helper that caps returndata
size.
