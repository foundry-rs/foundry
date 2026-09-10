# Uninitialized State Variables

**Severity**: `Med`
**ID**: `uninitialized-state`

## What it does

Reports state variables that are read but never assigned in the contract or its base contracts.
An assignment at the declaration or in a constructor satisfies the lint.

Contracts containing inline assembly are skipped. Writes through storage references may
still produce warnings, while read-only method calls on an uninitialized contract variable
can go unreported. Review initialization explicitly in these cases.

## Why is this bad?

A variable that is always read as its zero default is almost certainly a logic bug. Common
consequences include:

- Ownership checks that permanently pass or fail (`owner` is always `address(0)`).
- Token balances that always read as zero regardless of deposits.
- Flags and counters that never reflect actual contract state.

## Example

```solidity
contract Escrow {
    address public owner; // never set, always address(0)

    function withdraw() external {
        require(msg.sender == owner, "not owner"); // always fails
        payable(owner).transfer(address(this).balance);
    }
}
```

Use instead:

```solidity
contract Escrow {
    address public owner;

    constructor(address _owner) {
        owner = _owner; // initialized in constructor
    }

    function withdraw() external {
        require(msg.sender == owner, "not owner");
        payable(owner).transfer(address(this).balance);
    }
}
```
