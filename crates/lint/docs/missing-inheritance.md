# Missing inheritance

**Severity**: `Info`
**ID**: `missing-inheritance`

A contract that implements every external function of an interface but does not explicitly inherit
from it loses compile-time checks (the `override` keyword, signature/return-type validation, and
`type(I).interfaceId` recognition) and obscures intent for readers and tooling.

## What it does

For each non-interface contract `C` in the analyzed sources, this lint reports each interface `I`
where:

- `C` does **not** transitively inherit from `I`,
- `C` (including its inherited bases) implements every external selector exported by `I`, and
- no already-inherited base of `C` already covers all of `I`'s selectors.

When several candidate interfaces overlap (e.g. `IERC20` and `IERC20Metadata`), only the maximal
one is reported. "Interface-like" abstract contracts — those with no state, no constructor, no
modifier bodies, and no function bodies — are also treated as candidate interfaces, mirroring the
behavior of Slither's `missing-inheritance` detector.

## Why is this bad?

Explicit inheritance:

- documents which standards a contract claims to implement,
- enables the compiler to verify function signatures, visibility, mutability, and return types via
  `override`,
- makes `type(I).interfaceId` and ERC-165 introspection meaningful,
- makes refactors safer: changing the interface fails the build instead of silently drifting.

Implementing the API by coincidence (or by copy-paste) skips all of those checks.

## Example

### Bad

```solidity
interface ISomething {
    function f1() external returns (uint256);
}

contract Something {
    function f1() external returns (uint256) {
        return 42;
    }
}
```

### Good

```solidity
interface ISomething {
    function f1() external returns (uint256);
}

contract Something is ISomething {
    function f1() external override returns (uint256) {
        return 42;
    }
}
```
