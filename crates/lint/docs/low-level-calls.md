# Low-level calls

**Severity**: `Info`
**ID**: `low-level-calls`

## What it does

Warns whenever a contract uses a low-level call expression, even if the success return value is
captured and checked.

## Why restrict this?

Low-level calls bypass Solidity's normal ABI checks and function dispatch safety. They are also
easy to misuse because failures are reported through return values instead of automatically
reverting. Prefer typed interface calls when the target function is known.

Low-level calls are appropriate for dynamic dispatch, proxy forwarding, or custom failure
handling. Keep them when a typed call cannot express the intended behavior and review their
return-value handling before suppressing the lint.

## Example

```solidity
(bool ok, ) = target.call(abi.encodeCall(IReceiver.receiveMessage, (data)));
require(ok, "call failed");
```

Use instead:

```solidity
IReceiver(target).receiveMessage(data);
```
