# Prefer named struct fields

**Severity**: `Info`
**ID**: `named-struct-fields`

Flags struct construction expressions that pass fields positionally instead of by name.

## What it does

Reports `Struct(a, b, c)` style struct construction; suggests `Struct({ field1: a, field2: b,
field3: c })` instead.

## Why is this bad?

Positional struct construction is fragile: adding or reordering fields silently changes the
meaning of every existing call site. Named-field construction is self-documenting and resilient
to struct changes.

## Example

### Bad

```solidity
User memory u = User(addr, 100, true);
```

### Good

```solidity
User memory u = User({ wallet: addr, balance: 100, active: true });
```
