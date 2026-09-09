# Function init state

**Severity**: `Info`
**ID**: `function-init-state`

Flags state variables whose initializer depends on a non-pure function or on another state variable.

## What it does

Reports inline state-variable initializers that reference a non-constant state variable
or a non-pure function. Constants, pure functions, and assignments in the constructor body
are excluded.

## Why is this bad?

State variable initializers run at construction, before the constructor body, in base-to-derived
order. An initializer that reads another state variable or calls a function that does may observe
a default value before a later initializer or constructor assignment runs. Move dependent
initialization into the constructor when its ordering needs to be explicit.

## Example

```solidity
contract C {
    uint256 internal seed = 77;
    uint256 public value = compute(); // runs before the constructor sets anything

    function compute() internal view returns (uint256) {
        return seed * 2;
    }
}
```

Use instead:

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
