# Low-level calls

**Severity**: `Info`
**ID**: `low-level-calls`

Flags direct use of Solidity low-level calls (`call`, `delegatecall`, and `staticcall`).

## What it does

Warns whenever a contract uses a low-level call expression, even if the success return value is
captured and checked.

## Why is this bad?

Low-level calls bypass Solidity's normal ABI checks and function dispatch safety. They are also
easy to misuse because failures are reported through return values instead of automatically
reverting. Prefer typed interface calls when the target function is known.

## Example

### Bad

```solidity
(bool ok, bytes memory ret) = target.call(data);
require(ok, "call failed");
```

### Good

```solidity
IReceiver(target).receiveMessage(data);
```
