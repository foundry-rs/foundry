# Prefer named struct fields

**Severity**: `Info`
**ID**: `named-struct-fields`

Flags struct construction expressions that pass fields positionally instead of by name.

## What it does

Reports `Struct(a, b, c)` style struct construction; suggests `Struct({ field1: a, field2: b,
field3: c })` instead.

## Why restrict this?

Positional struct construction can become misleading when fields of the same type are reordered.
Named-field construction makes each value's role explicit and remains clear after such a reorder.
Adding a required field still requires updating either form of construction.

Positional construction can be clear for small, familiar structs or generated code. Keep it when
the field order is unambiguous and a project's convention favors the shorter form.

## Example

```solidity
User memory u = User(addr, 100, true);
```

Use instead:

```solidity
User memory u = User({ wallet: addr, balance: 100, active: true });
```
