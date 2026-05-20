# Use of tx.origin for authorization

**Severity**: `Med`
**ID**: `tx-origin`

Flags use of `tx.origin` inside authorization-like predicates such as `require`, `assert`, `if`,
`while`, and `for` conditions.

## What it does

Reports `tx.origin` reads when they are used as part of a guard condition. Plain reads outside of
guard predicates are not reported.

## Why is this bad?

`tx.origin` is the original externally owned account that started the whole transaction, not the
immediate caller. If authorization checks rely on `tx.origin`, a malicious contract can call the
protected contract while the legitimate owner is the transaction origin.

Use `msg.sender` for authorization checks instead.

## Example

### Bad

```solidity
require(tx.origin == owner, "not owner");
```

### Good

```solidity
require(msg.sender == owner, "not owner");
```
