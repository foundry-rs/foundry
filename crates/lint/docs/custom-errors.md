# Prefer custom errors over revert strings

**Severity**: `Gas`
**ID**: `custom-errors`

Flags `require(cond, "message")`, `revert("message")`, and `revert()` calls; suggests replacing
them with a `revert CustomError(...)`.

## What it does

Reports `require` calls whose second argument is a string literal, and `revert(...)` calls that
are either bare or have a string-literal argument.

## Why is this bad?

Custom errors:
- cost less gas than encoding/decoding a string,
- can carry typed parameters for richer diagnostics,
- shrink contract bytecode (string constants live in code).

Solidity 0.8.4+ supports custom errors natively.

## Example

### Bad

```solidity
require(amount > 0, "amount must be > 0");
revert("not authorized");
revert();
```

### Good

```solidity
error AmountZero();
error NotAuthorized();

if (amount == 0) revert AmountZero();
if (!authorized) revert NotAuthorized();
```

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
