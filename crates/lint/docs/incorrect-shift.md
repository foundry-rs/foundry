# Incorrect Yul shift order

**Severity**: `High`
**ID**: `incorrect-shift`

Flags Yul `shl` and `shr` calls whose operands appear to be reversed.

## What it does

Warns when the first argument to a Yul `shl` or `shr` call is dynamic and the second argument is a
literal. Yul shift calls take the shift amount first and the value second, so `shr(value, 8)`
shifts the literal `8` by `value`; the usual intended expression is `shr(8, value)`.

## Why is this bad?

Yul's shift argument order is easy to confuse with high-level Solidity operators. Reversing the
arguments silently changes which value is shifted and can produce incorrect arithmetic,
bit-packing, or bounds logic.

## Example

### Bad

```solidity
assembly {
    result := shl(value, 8)
    result := shr(add(value, 1), 16)
}
```

### Good

```solidity
assembly {
    result := shl(8, value)
    result := shr(16, add(value, 1))
    result := shl(8, 1) // both literals
}
```
