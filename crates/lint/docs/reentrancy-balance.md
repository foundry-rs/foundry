# Reentrancy through stale contract balance checks

**Severity**: `High`
**ID**: `reentrancy-balance`

Flags reentrant external calls between saving `address(this).balance` and checking the current
contract balance against that saved value.

## What it does

Reports public or external functions that save `address(this).balance`, make an external
call that permits reentry, and then check the current balance against the saved value.

This rule concerns the contract's own ETH balance, not token balances or other addresses.
View and static calls and calls where the callee receives at most 2,300 gas in total
(including any value-transfer stipend) are excluded.
A standard `nonReentrant` lock covering every mutable entry point can suppress the warning.

## Why is this bad?

A callback can re-enter the function several times before any invocation reaches its balance
check. Nested invocations can therefore share the same pre-call balance and make one payment appear
to satisfy several operations. A non-strict inequality does not prevent the attack because the
saved baseline, rather than the comparison operator, is stale.

## Example

```solidity
function mint(IPayer payer, uint256 amount) external {
    uint256 balanceBefore = address(this).balance;
    payer.pay();
    require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    _mint(msg.sender, amount);
}
```

Use instead:

```solidity
function mint(uint256 amount) external payable nonReentrant {
    require(msg.value >= amount, "insufficient payment");
    _mint(msg.sender, amount);
}
```
