# Unchecked low-level call

**Severity**: `High`
**ID**: `unchecked-call`

Flags low-level calls (`call`, `delegatecall`, `staticcall`, `callcode`) whose `success` return
value is ignored.

## What it does

Warns when the boolean returned by a low-level call is discarded — either because the return value
is not assigned or because only the `bytes memory` payload is used.

## Why is this bad?

Low-level calls do **not** revert when the callee fails; they silently return `false`. Ignoring
the success flag means a failed call is indistinguishable from a successful one, leading to bugs
where state is updated on the assumption that an external interaction succeeded.

## Example

### Bad

```solidity
target.call(data);                          // success ignored
(, bytes memory ret) = target.call(data);   // only payload kept
```

### Good

```solidity
(bool ok, ) = target.call(data);
require(ok, "call failed");
```
