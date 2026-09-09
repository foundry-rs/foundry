# Missing inheritance

**Severity**: `Info`
**ID**: `missing-inheritance`

A contract that implements every external function of an interface but does not explicitly inherit
from it loses compiler checks that its implementation conforms to the interface and obscures
intent for readers and tooling.

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
- makes the intended interface relationship explicit to readers and tooling,
- makes refactors safer: changing the interface fails the build instead of silently drifting.

Implementing the API by coincidence (or by copy-paste) skips all of those checks.

`type(I).interfaceId` is available independently of inheritance. Implementing ERC-165 support
still requires appropriate `supportsInterface` behavior; inheritance alone does not provide it.

## Example

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

Use instead:

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
