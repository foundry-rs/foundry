# calls-loop

## Description

Detects external calls made from inside loops.

## Why is this bad?

External calls inside loops can turn one reverting or gas-heavy callee into a denial-of-service for
the whole loop. This is especially risky for push-payment patterns where every recipient must accept
ETH or where every external contract must respond successfully before the function can complete.

## Example

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

## Recommended

Favor pull-based accounting: record the amount owed to each recipient, then let each recipient claim
their own funds in a separate transaction.
