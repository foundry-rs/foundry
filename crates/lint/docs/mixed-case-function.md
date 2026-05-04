# Function names should use mixedCase

**Severity**: `Info`
**ID**: `mixed-case-function`

Flags function names that do not follow `mixedCase`.

## What it does

Reports functions whose names contain underscores, start with an uppercase letter, or otherwise
deviate from `mixedCase`. Test functions starting with `test`, `invariant_`, or `statefulFuzz`
and user-defined patterns (e.g. `ERC20`) are exempted.

## Why is this bad?

The Solidity style guide recommends `mixedCase` for function names. Consistent style makes call
sites uniform, helps editor tooling, and reduces friction in code review.

## Example

### Bad

```solidity
function get_balance() external view returns (uint256);
function GetBalance()  external view returns (uint256);
```

### Good

```solidity
function getBalance() external view returns (uint256);
```
