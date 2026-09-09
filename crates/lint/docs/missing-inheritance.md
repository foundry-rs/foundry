# Missing inheritance

**Severity**: `Info`
**ID**: `missing-inheritance`

## What it does

Reports contracts that implement an interface's external functions without inheriting it.
An already-inherited base that provides the interface's functions satisfies the lint.
Abstract contracts containing only interface declarations are also considered.

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
