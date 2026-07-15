# Reentrancy through stale contract balance checks

**Severity**: `High`
**ID**: `reentrancy-balance`

Flags reentrant external calls between saving `address(this).balance` and checking the current
contract balance against that saved value.

## What it does

Warns when a public or external entry point saves `address(this).balance` in a local value, performs
an external call that can forward enough gas to re-enter, and then compares the saved value with a
fresh contract-balance read in a `require`, `assert`, or exiting branch. Casts, tuple assignments,
derived locals, fresh post-call balance locals, internal helper parameters and returns, and
modifiers are tracked when their bodies are available.

This detector intentionally covers native ETH held by the current contract. It is not the later
token `balanceOf(address)` detector with a similar name. It does not report token balances,
balances of other addresses, mutable state or storage baselines, view or static calls, calls capped
at the 2,300-gas stipend or less, checks on directly mutually exclusive branches, baselines
overwritten after the call, or expressions that do not compare a fresh contract-balance read with
the stale local value.

## Why is this bad?

A callback can re-enter the function several times before any invocation reaches its balance
check. Nested invocations can therefore share the same pre-call balance and make one payment appear
to satisfy several operations. A non-strict inequality does not prevent the attack because the
saved baseline, rather than the comparison operator, is stale.

## Example

### Bad

```solidity
function mint(IPayer payer, uint256 amount) external {
    uint256 balanceBefore = address(this).balance;
    payer.pay();
    require(address(this).balance >= balanceBefore + amount, "insufficient payment");
    _mint(msg.sender, amount);
}
```

### Good

```solidity
function mint(uint256 amount) external payable nonReentrant {
    require(msg.value >= amount, "insufficient payment");
    _mint(msg.sender, amount);
}
```
