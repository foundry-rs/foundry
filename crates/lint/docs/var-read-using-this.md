# State variable read via `this`

**Severity**: `Gas`
**ID**: `var-read-using-this`

Flags reads of the contract's own state through `this.X(...)`. Calling a public state-variable
getter or any `view`/`pure` function via `this` performs an external `STATICCALL` to the same
address, paying the call overhead for data that could be read directly.

## What it does

Reports `this.<name>(<args>)` calls where `<name>` resolves (via overload resolution by arity) to
a function reachable on the contract's external interface (`public` or `external`) whose state
mutability is `view` or `pure`. This includes:

- The auto-generated getter for any `public` state variable (simple variables, mappings, arrays).
- Any inherited `public`/`external` `view`/`pure` function declared in a base contract.

When the offending call is the auto-generated getter for a state variable, the lint emits a code
fix:

- Simple state variable: `this.foo()` → `foo` (machine-applicable).
- Mapping/array getter: `this.m(k)` → `m[k]` (`maybe-incorrect`; double-check the rewrite).

Calls that carry call options (e.g. `this.foo{gas: 1000}()`) are still flagged, but no fix is
suggested — the developer is intentionally reaching for the external-call machinery.

## Why is this bad?

Each `this.X(...)` call compiles to a `STATICCALL` to the contract's own address. That costs a
fixed amount of gas, plus the encoding/decoding of arguments and return data, in addition to the
storage read itself. Reading the variable directly skips the call entirely.

## Example

### Bad

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

### Good

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

## Notes

This is a `Gas`-severity lint and is **not** applied to test or script files.

For `external view`/`pure` functions, calling them via `this` from inside the contract is the
only in-contract syntax that resolves; the recommended fix is to extract the body into an
`internal` helper that both the `external` entry point and the local caller invoke.
