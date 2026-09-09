# Literal instead of constant

**Severity**: `Info`
**ID**: `literal-instead-of-constant`

## What it does

Reports repeated number, address, or hex-string values within a contract's executable code.
Equivalent spellings, such as `100` and `0x64`, count as the same value.

`0`, `1`, and `2`, plain array indices, slice bounds, shift amounts, type annotations,
and inline assembly are excluded. Repetitions in separate contracts are checked separately.

## Why restrict this?

A repeated literal is a configuration value the contract never named: each copy can drift independently on the next edit, and the reader has no word for what the value means. A named constant gives it one definition and one meaning.

Equal literals can have unrelated meanings. Keep separate literals when sharing one constant
would incorrectly couple independently changing values, and suppress the reported occurrences.

## Example

```solidity
function deposit(uint256 amount) external {
    require(amount >= 500, "too small");
    // ...
}

function fee() public pure returns (uint256) {
    return 500;
}
```

Use instead:

```solidity
uint256 private constant MIN_DEPOSIT = 500;

function deposit(uint256 amount) external {
    require(amount >= MIN_DEPOSIT, "too small");
    // ...
}

function fee() public pure returns (uint256) {
    return MIN_DEPOSIT;
}
```
