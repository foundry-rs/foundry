# Function selector collision

**Severity**: `High`
**ID**: `function-selector-collision`

## What it does

Reports different proxy and implementation function signatures with the same four-byte
selector. Identical signatures are not reported.

This lint covers fallbacks that directly delegate the full calldata to a target with an
explicit contract or interface type. For assembly-based proxies or targets stored as
`address`, check the pair explicitly with
`forge selectors collision <proxy> <implementation>`.

## Why is this bad?

A proxy dispatches its own external functions before its fallback. If a proxy function and an implementation function have the same selector, calls intended for the implementation execute the proxy function instead. The implementation function becomes unreachable through the proxy and may produce unexpected state changes or access-control behavior.

## Example

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

Use instead:

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
