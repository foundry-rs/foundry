# Prefer named struct fields

**Severity**: `Info`
**ID**: `named-struct-fields`

## What it does

Reports `Struct(a, b, c)` style struct construction; suggests `Struct({ field1: a, field2: b,
field3: c })` instead.

## Why restrict this?

Positional struct construction can become misleading when fields of the same type are reordered.
Named-field construction makes each value's role explicit and remains clear after such a reorder.

Positional construction can still be clear for small, familiar structs.

## Example

```solidity
User memory u = User(addr, 100, true);
```

Use instead:

```solidity
User memory u = User({ wallet: addr, balance: 100, active: true });
```
