# Inconsistent type names

**Severity**: `Low`
**ID**: `inconsistent-type-names`

Flags mixed use of the equivalent `uint`/`uint256` or `int`/`int256` type spellings within one contract.

## What it does

Reports a variable declaration that uses `uint` when another declaration in the same directly
declared contract uses `uint256`, and likewise reports `int` when the contract also uses `int256`.
Only the shorthand declaration is reported because the explicit 256-bit spelling is preferred.

The lint uses resolved HIR variable ownership and parsed elementary types rather than searching
source text for type-like names. State variables, struct fields, event and error parameters,
function parameters and returns, local variables, try/catch variables, and function-type
parameters and returns are declarations in scope. Array element and mapping key/value types are
inspected recursively. A declaration such as `mapping(uint => uint256)` is therefore inconsistent
by itself and produces one diagnostic for the declaration.

Consistency is evaluated separately for each directly declared contract. Types declared in a base
contract or a different contract do not affect a child, and type spellings in casts, `type(...)`
expressions, `using ... for` directives, or user-defined value type definitions are not variable
declarations and do not affect the result. A contract that consistently uses only `uint` and `int`
is not reported, though explicit sizes remain preferable.

## Why is this bad?

`uint` and `uint256` compile to the same type, as do `int` and `int256`, so mixing their spellings
does not change runtime behavior. It does make the code less consistent and can make readers wonder
whether an omitted size was intentional. Using the explicit spelling throughout removes that
ambiguity.

## Example

### Bad

```solidity
contract Vault {
    uint public shares;
    uint256 public assets;
}
```

### Good

```solidity
contract Vault {
    uint256 public shares;
    uint256 public assets;
}
```
