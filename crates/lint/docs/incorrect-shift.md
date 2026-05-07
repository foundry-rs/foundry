# Incorrect shift order

**Severity**: `High`
**ID**: `incorrect-shift`

Flags shift operations where a literal appears on the left and a non-literal on the right, which
is almost always the wrong operand order.

## What it does

Warns when the left-hand operand of `<<` or `>>` is a numeric literal and the right-hand operand
is a non-literal expression (e.g. a variable, function call, or composite expression).

## Why is this bad?

Shift expressions like `2 << x` are usually a typo for `x << 2`. In the former, the *value being
shifted* is a tiny constant and the *shift amount* is dynamic — almost never the intended
behavior, and a known source of bugs in production contracts.

## Example

### Bad

```solidity
result = 2 << stateValue;        // shift amount comes from state
result = 8 >> localValue;        // shift amount comes from a local
result = 16 << (stateValue + 1); // shift amount is a dynamic expression
```

### Good

```solidity
result = stateValue << 2;
result = localValue >> 3;
result = stateValue << localShiftAmount;
result = 1 << 8; // both literals — fine
```
