# Delegatecall inside payable loop

**Severity**: `Low`
**ID**: `delegatecall-loop`

Flags `delegatecall` operations inside loops in externally callable payable functions.

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

### Bad

```solidity
function batch(address[] calldata receivers) external payable {
    for (uint256 i; i < receivers.length; ++i) {
        address(this).delegatecall(abi.encodeWithSignature("credit(address)", receivers[i]));
    }
}
```

### Good

```solidity
function batch(address[] calldata receivers) external payable {
    uint256 share = msg.value / receivers.length;
    for (uint256 i; i < receivers.length; ++i) {
        _credit(receivers[i], share);
    }
}
```

## Notes

Review each occurrence manually. If `delegatecall` is required, ensure delegated code cannot
reuse `msg.value` or unexpectedly modify caller storage across loop iterations.
