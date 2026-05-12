# Dead code

**Severity**: `CodeSize`
**ID**: `dead-code`

Flags internal and private functions that are not reachable from contract entry points or
construction-time initializers.

## What it does

Reports implemented internal or private functions that are never reachable from public/external
functions, constructors, fallback/receive functions, modifiers used by reachable functions, or state
variable initializers. Public and external functions are treated as entry points. Abstract
declarations, library functions, and virtual base implementations that are overridden are skipped.

## Why is this bad?

Dead functions increase bytecode size and make review harder. Removing them reduces deployment cost,
keeps contracts easier to audit, and avoids carrying stale implementation paths.

## Example

### Bad

```solidity
contract C {
    function run() external {}

    function unused() internal pure returns (uint256) {
        return 1;
    }
}
```

### Good

```solidity
contract C {
    function run() external pure returns (uint256) {
        return helper();
    }

    function helper() internal pure returns (uint256) {
        return 1;
    }
}
```

## Notes

This is a `CodeSize`-severity lint and is **not** applied to test or script files.
