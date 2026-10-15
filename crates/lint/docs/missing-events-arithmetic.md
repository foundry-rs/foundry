# Missing events arithmetic

**Severity**: `Low`
**ID**: `missing-events-arithmetic`

## What it does

Flags protected public or external functions that update scalar integer state variables used in
arithmetic by an unprotected entry point without emitting an event. Updates include
assignments from function input and arithmetic changes.

Constructors, unprotected setters, fixed-value assignments, mappings, and arrays are excluded.

## Why is this bad?

Off-chain monitors, users, and auditors often rely on events to track changes to critical contract
parameters such as prices, fees, caps, and rates. If a protected function silently changes a
parameter used in calculations, downstream behavior can change without an easy audit trail.

## Example

```solidity
function setBuyPrice(uint256 newBuyPrice) external onlyOwner {
    buyPrice = newBuyPrice;
}
```

Use instead:

```solidity
event BuyPriceUpdated(uint256 newBuyPrice);

function setBuyPrice(uint256 newBuyPrice) external onlyOwner {
    buyPrice = newBuyPrice;
    emit BuyPriceUpdated(newBuyPrice);
}
```
