# Return bomb

**Severity**: `Low`
**ID**: `return-bomb`

Flags external calls that set an explicit gas limit while copying unbounded dynamic returndata.

## What it does

Detects low-level `call`, `delegatecall`, and `staticcall` expressions that specify `{gas: ...}`.
Solidity copies the full returndata for these calls even when the second tuple element is ignored.
It also detects high-level external calls with `{gas: ...}` that consume dynamically encoded return
values such as `bytes`, `string`, dynamic arrays, or structs containing dynamic fields.

## Why is this bad?

The gas option limits gas forwarded to the callee, but copying returndata into memory is paid by
the caller after the call returns. A malicious callee can return or revert with large returndata,
causing the caller to run out of gas while implicitly copying the result.

## Example

### Bad

```solidity
function callTarget(address target, bytes memory payload, uint256 gasLimit) external {
    (bool ok, ) = target.call{gas: gasLimit}(payload);
    require(ok);
}
```

### Good

```solidity
function callTarget(address target, bytes memory payload, uint256 gasLimit) external {
    bool ok;
    assembly {
        ok := call(gasLimit, target, 0, add(payload, 0x20), mload(payload), 0, 0)
    }
    require(ok);
}
```

Ignoring the second tuple element does not avoid the copy. If the returndata is needed, copy only a
bounded number of bytes with a helper that caps returndata size.
