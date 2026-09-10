# State variable read via `this`

**Severity**: `Gas`
**ID**: `var-read-using-this`

## What it does

Reports calls through `this` to the contract's own public variable getters and `view`
or `pure` functions, including inherited functions.

Read state directly where possible: use `foo` instead of `this.foo()`, or `m[k]` instead
of `this.m(k)`. Check that the replacement preserves the getter's return value and any
intentional external-call behavior.

Struct getters return selected fields rather than the struct itself, and a local variable
may shadow the state variable. Calls with explicit gas options receive no replacement;
calls used as the target of `try` are excluded because `try` requires an external call.

## Why is this bad?

Each `this.X(...)` call compiles to a `STATICCALL` to the contract's own address. That costs a
fixed amount of gas, plus the encoding/decoding of arguments and return data, in addition to the
storage read itself. Reading the variable directly skips the call entirely.

## Example

```solidity
contract C {
    uint256 public counter;
    mapping(uint256 => address) public owners;

    function readDirect() external view returns (uint256, address) {
        // Each `this.X` performs an unnecessary STATICCALL.
        return (this.counter(), this.owners(0));
    }
}
```

Use instead:

```solidity
contract C {
    uint256 public counter;
    mapping(uint256 => address) public owners;

    function readDirect() external view returns (uint256, address) {
        // Direct storage reads — no external call.
        return (counter, owners[0]);
    }
}
```

For `external view`/`pure` functions, calling them via `this` from inside the contract is the
only in-contract syntax that resolves; the recommended fix is to extract the body into an
`internal` helper that both the `external` entry point and the local caller invoke.
