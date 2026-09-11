# Unused return value

**Severity**: `Med`
**ID**: `unused-return`

## What it does

Detects high-level external calls, including external library calls, that return one or more
values when the entire result is discarded or any slot of a tuple return is omitted. ERC20
`transfer` and `transferFrom` are excluded as they are handled by the separate
`erc20-unchecked-transfer` lint. Internal calls, including library calls and
qualified base-contract calls, are excluded.

## Why is this bad?

Discarding a return value can hide an application-level failure reported in that value or ignore
the result of a query. A high-level external call still propagates a revert even when its return
value is discarded; this lint concerns information returned by successful calls.

## Example

```solidity
interface IOracle {
    function getPrice(address token) external returns (uint256);
}

contract Example {
    IOracle oracle;

    function updatePrice(address token) external {
        oracle.getPrice(token); // return value silently discarded
    }
}
```

Use instead:

```solidity
interface IOracle {
    function getPrice(address token) external returns (uint256);
}

contract Example {
    IOracle oracle;
    uint256 public lastPrice;

    function updatePrice(address token) external {
        lastPrice = oracle.getPrice(token);
    }
}
```
