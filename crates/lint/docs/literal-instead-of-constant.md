# Literal instead of constant

**Severity**: `Info`
**ID**: `literal-instead-of-constant`

Flags literal values appearing more than once in the executable bodies of a contract.

## What it does

Reports every occurrence of a number, address or hex string literal whose value appears more than once in the bodies a contract declares: functions, constructors, modifiers, `receive` and `fallback`. Values are compared semantically, so `100`, `0x64` and `1e2` are one value, and `1 ether` equals `1e18`; Aderyn's detector of the same name compares the source spellings instead. Out of scope: `0`, `1` and `2` (structural values), bare literals indexing an array-like value or bounding a slice (positional), string and bool literals, single occurrences, and repetitions split across two contracts. Mapping keys count, they are configuration data rather than positions, and so do Yul `case` labels.

## Why is this bad?

A repeated literal is a configuration value the contract never named: each copy can drift independently on the next edit, and the reader has no word for what the value means. A named constant gives it one definition and one meaning.

## Example

### Bad

```solidity
function deposit(uint256 amount) external {
    require(amount >= 500, "too small");
    ...
}

function fee() public pure returns (uint256) {
    return 500;
}
```

### Good

```solidity
uint256 private constant MIN_DEPOSIT = 500;

function deposit(uint256 amount) external {
    require(amount >= MIN_DEPOSIT, "too small");
    ...
}

function fee() public pure returns (uint256) {
    return MIN_DEPOSIT;
}
```
