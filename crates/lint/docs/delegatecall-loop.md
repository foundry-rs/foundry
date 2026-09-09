# `delegatecall` inside a payable loop

**Severity**: `Low`
**ID**: `delegatecall-loop`

## What it does

Reports `delegatecall` expressions that appear in the body of a `for`, `while`, or `do while`
loop when the enclosing function is `public payable` or `external payable`.

## Why is this bad?

`delegatecall` executes another contract's code in the caller's storage context and preserves
the original `msg.sender` and `msg.value`. In a payable function, a loop can therefore expose the
same `msg.value` to multiple delegatecalls even though Ether was only transferred once.

If the delegated code accounts for `msg.value`, one transaction can credit the same payment
multiple times or repeatedly mutate the caller's storage in unexpected ways.

## Example

```solidity
function batch(address[] calldata receivers) external payable {
    for (uint256 i; i < receivers.length; ++i) {
        address(this).delegatecall(abi.encodeWithSignature("credit(address)", receivers[i]));
    }
}
```

Use instead:

For an equal split, reject an empty recipient list and choose how to handle any remainder. This
example accepts only exactly divisible payments; other APIs may explicitly refund or account for
the remainder.

```solidity
function batch(address[] calldata receivers) external payable {
    require(receivers.length != 0, "no receivers");
    require(msg.value % receivers.length == 0, "unequal split");
    uint256 share = msg.value / receivers.length;
    for (uint256 i; i < receivers.length; ++i) {
        _credit(receivers[i], share);
    }
}
```
