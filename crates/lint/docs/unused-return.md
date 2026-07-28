# Unused return value

**Severity**: `Med`
**ID**: `unused-return`

Flags external calls whose return value is discarded, which often indicates a logic bug where the
result of a computation or state query is silently ignored.

## What it does

Detects high-level external calls (member calls on contract-typed variables or interface-cast
addresses) that return one or more values when the entire result is discarded or any slot of a
tuple return is omitted. ERC20 `transfer` and `transferFrom` are excluded as they are handled by
the separate `erc20-unchecked-transfer` lint.

## Why is this bad?

Discarding a return value can mask failures and incorrect assumptions. For example, ignoring the
result of an oracle query or a state-mutating helper means the caller proceeds as if the call
succeeded or that the value is irrelevant, both of which may be bugs.

## Example

### Bad

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

### Good

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
