# Function init state

**Severity**: `Info`
**ID**: `function-init-state`

Flags state variables whose initializer depends on a non-pure function or on another state variable.

## What it does

Reports a state variable whose inline initializer references a non-constant state variable or a non-pure function (called or referenced, including inside the arguments of a nested call). A public variable referenced through its synthesized getter counts as a read of the variable itself, so references to public constants stay clean. This mirrors Slither's `function-init-state` detector.

References to constants, calls to pure functions and plain literal expressions are fine, and assignments made inside the constructor body are out of scope.

## Why is this bad?

State variable initializers run at construction, before the constructor body, in base-to-derived order. An initializer that reads another state variable or calls a function that does may observe default values or an ordering the author did not intend, so the computed value is rarely the expected one, and silently so.

## Example

### Bad

```solidity
contract C {
    uint256 internal seed = 77;
    uint256 public value = compute(); // runs before the constructor sets anything

    function compute() internal view returns (uint256) {
        return seed * 2;
    }
}
```

### Good

```solidity
contract C {
    uint256 internal seed = 77;
    uint256 public value;

    constructor() {
        value = compute();
    }

    function compute() internal view returns (uint256) {
        return seed * 2;
    }
}
```
