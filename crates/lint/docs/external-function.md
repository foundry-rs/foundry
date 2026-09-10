# External Function

**Severity**: `Gas`
**ID**: `external-function`

## What it does

Flags implemented `public` functions with reference-type `memory` parameters that are
never called internally and do not modify their parameters. Overrides are excluded.
Internal references and `super` calls count against the selected overload and inherited target.

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
