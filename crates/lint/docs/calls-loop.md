# External call inside a loop

**Severity**: `Low`
**ID**: `calls-loop`

Flags external calls made from inside loops, including calls reached through modifiers or internal
helper functions.

## What it does

Reports high-level contract calls, low-level `call`/`delegatecall`/`staticcall`, Ether
`send`/`transfer`, external self-calls through `this`, and contract creation inside a loop. Internal
and private library calls and `super` dispatch are not treated as external calls.

## Why is this bad?

External calls inside loops can turn one reverting or gas-heavy callee into a denial-of-service for
the whole loop. This is especially risky for push-payment patterns where every recipient must accept
ETH or where every external contract must respond successfully before the function can complete.

## Example

### Bad

```solidity
contract Payouts {
    address payable[] recipients;

    function payAll() external payable {
        for (uint256 i; i < recipients.length; ++i) {
            recipients[i].transfer(1 ether);
        }
    }
}
```

### Good

```solidity
contract Payouts {
    mapping(address recipient => uint256 amount) public claimable;

    function claim() external {
        uint256 amount = claimable[msg.sender];
        claimable[msg.sender] = 0;
        payable(msg.sender).transfer(amount);
    }
}
```
