# Prefer custom errors over revert strings

**Severity**: `Gas`
**ID**: `custom-errors`

Flags `require(cond)`, `require(cond, "message")`, `revert("message")`, and `revert()` calls;
suggests replacing them with a `revert CustomError(...)`.

## What it does

Reports `require` calls with no reason or whose second argument is a string literal, and
`revert(...)` calls that are either bare or have a string-literal argument.

## Why is this bad?

Custom errors can reduce the bytecode and revert-data costs of descriptive strings, and can carry
typed parameters for richer diagnostics. Exact savings depend on the error and compiler settings.

A bare `require(cond)` or `revert()` returns no error data. Replacing it with a custom error adds
diagnostic data and can increase gas costs; this is a clarity tradeoff, not a gas optimization.
Keep an intentionally empty revert when that behavior is part of the contract's interface.

Solidity 0.8.4+ supports custom errors natively.

## Example

```solidity
function validate(uint256 amount) internal pure {
    require(amount > 0, "amount must be > 0");
}

function fail() internal pure {
    revert("not authorized");
}
```

Use instead:

```solidity
error AmountZero();
error NotAuthorized();

function validate(uint256 amount) internal pure {
    if (amount == 0) revert AmountZero();
}

function fail() internal pure {
    revert NotAuthorized();
}
```

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.
