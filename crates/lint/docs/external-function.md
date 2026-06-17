# External Function

**Severity**: `Gas`
**ID**: `external-function`

`public` functions that are never called from inside the contract (or any of its
derivatives) can be declared `external`. External functions read their reference-type
arguments directly from `calldata` instead of copying them into `memory`, which saves
gas at every call site.

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

Calling a `public` function from outside the contract is more expensive than calling
the equivalent `external` function:

- Each reference-type parameter is copied from `calldata` into `memory` before the
  function body executes, even though `external`-only callers never need that copy.
- The opcode shim that allows the function to be called both internally and externally
  adds a few bytes of bytecode and an extra branch on every entry.

When the function is never called internally, switching `public` to `external` removes
both costs at no semantic change.

## Example

### Bad

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

### Good

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
