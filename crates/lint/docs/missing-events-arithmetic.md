# Missing events arithmetic

**Severity**: `Low`
**ID**: `missing-events-arithmetic`

Flags protected entry-point functions that update tainted integer state used in arithmetic by an
unprotected function without emitting an event.

## What it does

This lint looks for scalar `int`/`uint` state variables that are:

- written by a public or external state-mutating function with access control,
- assigned from function input, directly or through local aliases/internal helpers, or changed with
  an arithmetic assignment or increment/decrement,
- used in arithmetic by an unprotected public or external function, and
- changed along a path that does not emit an event.

It intentionally skips fixed-value writes, constructors, unprotected setters, mappings/arrays, and
update paths that emit an event directly or through an internal helper. Those limits keep the rule
focused on Slither's low-severity `events-maths` case while avoiding common false positives.

## Why is this bad?

Off-chain monitors, users, and auditors often rely on events to track changes to critical contract
parameters such as prices, fees, caps, and rates. If a protected function silently changes a
parameter used in calculations, downstream behavior can change without an easy audit trail.

## Example

### Bad

```solidity
function setBuyPrice(uint256 newBuyPrice) external onlyOwner {
    buyPrice = newBuyPrice;
}
```

### Good

```solidity
event BuyPriceUpdated(uint256 newBuyPrice);

function setBuyPrice(uint256 newBuyPrice) external onlyOwner {
    buyPrice = newBuyPrice;
    emit BuyPriceUpdated(newBuyPrice);
}
```
