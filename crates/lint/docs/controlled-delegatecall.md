# Controlled delegatecall

**Severity**: `High`
**ID**: `controlled-delegatecall`

`delegatecall` executes code from another address in the caller's storage and execution context.
This lint detects delegatecall targets that are not provably trusted, including targets that can be
influenced by users, mutable storage, or constructor-provided values.

## What it does

This lint flags `delegatecall` where the destination is not provably a trusted literal, constant,
zero address, or `address(this)`. It tracks local aliases, simple helper returns, local equality
guards against trusted values, and modifier guards that refine the target argument before `_`.
Function parameters, `msg.sender`, mutable state variables, mapping and array reads, local aliases of
those values, and all immutables are treated conservatively as untrusted.

This lint does not attempt to prove trust through access-control modifiers such as `onlyOwner`,
role checks, allowlist mappings, implementation-slot reads, assembly, external/library predicate
helpers, codehash checks, or constructor-initialized immutables. Those patterns may be safe in a
specific system, but they are intentionally outside this lint's proof model and can still warn.

## Why is this bad?

A controlled delegatecall target can run arbitrary code against the calling contract's storage. An
attacker-controlled implementation can overwrite state, bypass invariants, drain funds, or destroy
the contract.

## Example

### Bad

```solidity
contract Delegatecall {
    function delegate(address target, bytes calldata data) external {
        target.delegatecall(data);
    }
}
```

### Good

```solidity
contract Delegatecall {
    address public constant IMPLEMENTATION = 0x000000000000000000000000000000000000dEaD;

    function delegate(bytes calldata data) external {
        IMPLEMENTATION.delegatecall(data);
    }
}
```

## Configuration

This lint has no additional configuration.
