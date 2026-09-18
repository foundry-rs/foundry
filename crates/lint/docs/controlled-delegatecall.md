# Controlled `delegatecall`

**Severity**: `High`
**ID**: `controlled-delegatecall`

## What it does

Flags `delegatecall` targets other than a trusted literal, constant, zero address, or
`address(this)`.

Warnings can remain for owner-controlled proxies, allowlisted implementations, and
constructor-initialized immutable targets. Review how the target is authorized before
suppressing the lint; these patterns are not automatically unsafe.

## Why is this bad?

A controlled delegatecall target can run arbitrary code against the calling contract's storage. An
attacker-controlled implementation can overwrite state, bypass invariants, drain funds, or destroy
the contract.

## Example

```solidity
contract Delegatecall {
    function delegate(address target, bytes calldata data) external {
        target.delegatecall(data);
    }
}
```

Use instead:

```solidity
contract Delegatecall {
    address public constant IMPLEMENTATION = 0x000000000000000000000000000000000000dEaD;

    function delegate(bytes calldata data) external {
        IMPLEMENTATION.delegatecall(data);
    }
}
```
