# External Function

**Severity**: `Gas`
**ID**: `external-function`

`public` functions that are never called from inside the contract (or any of its
derivatives) may be candidates for `external` visibility and `calldata` parameters. Reading
reference-type arguments directly from `calldata` can avoid a copy into `memory`.

## What it does

Flags a `public` function declaration when **all** of the following hold:

- The function is `public` (not `external`, `internal`, or `private`).
- It is an ordinary function (not a constructor, fallback, receive, or modifier).
- It has at least one parameter that is a reference type (`struct`, array, `bytes`, or
  `string`) currently located in `memory`.
- It is not an `override` of another function (the base must be migrated first).
- It has a body (not abstract or interface-only).
- It does not write to any of its parameters inside the body.
- It is never called from inside the contract or any contract that derives from it,
  whether directly (`foo()`), via `super.foo(...)`, or via a function-pointer reference
  (`fn = foo;`).

The lint runs in the `Gas` severity bucket and is automatically skipped on Foundry
test and script files.

## Why is this bad?

Reference-type parameters declared `memory` require a copy when read from external call data.
Using `calldata` can avoid that copy when the function only reads the parameters. Changing
visibility alone does not change their data location, and modern Solidity also permits `calldata`
on public functions. Verify callers and inheritance before removing the internal entry point,
and measure savings with the project's compiler settings.

## Example

```solidity
contract Vault {
    mapping(address => uint256) public balances;

    function deposit(address[] memory accounts, uint256[] memory amounts) public {
        for (uint256 i = 0; i < accounts.length; i++) {
            balances[accounts[i]] += amounts[i];
        }
    }
}
```

`deposit` is never called from inside `Vault`, but its `memory` arrays force an
unnecessary calldata-to-memory copy on every external call.

Use instead:

```solidity
contract Vault {
    mapping(address => uint256) public balances;

    function deposit(address[] calldata accounts, uint256[] calldata amounts) external {
        for (uint256 i = 0; i < accounts.length; i++) {
            balances[accounts[i]] += amounts[i];
        }
    }
}
```

When you migrate `public` to `external`, also change reference-type parameters from
`memory` to `calldata` to capture the full gas saving.
