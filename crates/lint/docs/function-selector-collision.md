# Function selector collision

**Severity**: `High`
**ID**: `function-selector-collision`

Flags different proxy and implementation function signatures that produce the same four-byte selector and can route implementation calls to the proxy instead.

## What it does

Solidity already rejects selector collisions within one contract and its inheritance tree, so this lint focuses on the separate APIs that coexist at a proxy address. It identifies a proxy/implementation pair when a concrete contract's fallback directly calls `delegatecall(msg.data)` on an address converted from a statically typed contract or interface value. It compares the proxy's effective external API with the target type's effective external API and reports different signatures with the same four-byte selector. Identical signatures are not reported because they are function shadowing rather than a hash collision.

The explicit source-level target type is required to keep the check conservative. The lint does not compare unrelated contracts across the project and intentionally does not infer targets stored only as `address`, read from EIP-1967 slots, used from inline assembly, or hidden behind helper functions. For those common proxy forms, use `forge selectors collision <proxy> <implementation>` to designate the pair explicitly.

## Why is this bad?

A proxy dispatches its own external functions before its fallback. If a proxy function and an implementation function have the same selector, calls intended for the implementation execute the proxy function instead. The implementation function becomes unreachable through the proxy and may produce unexpected state changes or access-control behavior.

## Example

### Bad

```solidity
interface IImplementation {
    function gsf() external;
}

contract Proxy {
    IImplementation internal implementation;

    // tgeo() and gsf() both have selector 0x67e43e43.
    function tgeo() external {}

    fallback() external payable {
        address(implementation).delegatecall(msg.data);
    }
}
```

### Good

```solidity
interface IImplementation {
    function gsf() external;
}

contract Proxy {
    IImplementation internal implementation;

    function proxyAdminAction() external {}

    fallback() external payable {
        address(implementation).delegatecall(msg.data);
    }
}
```
