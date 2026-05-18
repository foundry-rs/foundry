# Uninitialized local variable

**Severity**: `Med`
**ID**: `uninitialized-local`

Flags local variables that are declared without an initializer and then read before any assignment. In Solidity, uninitialized value-type locals silently default to zero (`address` -> `address(0)`, `uint` -> `0`, `bool` -> `false`), so this is almost always a logic bug rather than intentional behavior.

## What it does

Reports any local variable of `VarKind::Statement` (i.e., a variable declared inside a function body, not a parameter or state variable) whose first use is a read and which has never been explicitly assigned prior to that read on at least one execution path.

## Why is this bad?

Reading an uninitialized variable means the code silently depends on a language-level zero-default rather than an explicit value chosen by the developer. Common consequences include:

- Sending ETH to `address(0)` and burning it permanently (`address payable to; to.transfer(...)`).
- Arithmetic operating on an implicit `0` that bypasses guards or produces unexpected results.
- Returning a meaningless zero from a function whose caller assumes a real value.

The Solidity compiler does not warn about this; only static analysis catches it.

## Example

### Bad

```solidity
// `to` is never assigned, defaults to address(0), burning all ETH.
function withdraw() public {
    address payable to;
    to.transfer(address(this).balance);
}

// `amount` is never assigned, silently returns 0.
function getAmount() public pure returns (uint256) {
    uint256 amount;
    return amount;
}
```

### Good

```solidity
function withdraw(address payable recipient) public {
    address payable to = recipient;
    to.transfer(address(this).balance);
}

function getAmount(uint256 value) public pure returns (uint256) {
    uint256 amount = value;
    return amount;
}
```
