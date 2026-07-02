# Mapping deletion

**Severity**: `Med`
**ID**: `mapping-deletion`

Flags `delete` applied to a value whose type contains a `mapping`.

## What it does

Reports `delete x` when `x` is a struct or array whose type holds a `mapping`, directly or through a
nested struct or array. Deleting a whole mapping is not valid Solidity, but deleting a container of
one compiles and silently leaves the mapping's entries in place.

## Why is this bad?

`delete` resets a storage value to its default by zeroing each member, but it cannot iterate a
mapping's keys. The mapping's existing entries stay in storage, so the value is only partially
cleared. This is a common source of accounting and access-control bugs: the struct looks reset while
stale balances or flags remain reachable.

## Example

### Bad

```solidity
struct Account {
    uint256 total;
    mapping(address => uint256) balances;
}

mapping(uint256 => Account) accounts;

function reset(uint256 id) external {
    delete accounts[id]; // `total` is zeroed, but `balances` keeps every entry
}
```

### Good

```solidity
function reset(uint256 id, address[] calldata holders) external {
    Account storage acc = accounts[id];
    for (uint256 i; i < holders.length; ++i) {
        delete acc.balances[holders[i]];
    }
    acc.total = 0;
}
```
