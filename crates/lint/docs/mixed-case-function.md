# Function names should use `mixedCase`

**Severity**: `Info`
**ID**: `mixed-case-function`

Flags function names that do not follow `mixedCase`.

## What it does

Reports functions whose names contain embedded underscores, start with an uppercase letter, or
otherwise deviate from `mixedCase`. Leading and trailing underscores are preserved, and
single-character names are not checked. Test functions starting with `test`, `invariant_`, or
`statefulFuzz`, configured uppercase patterns (for example, `ERC20`), and external constant-style
getters are exempted.

## Why restrict this?

The Solidity style guide recommends `mixedCase` for function names. Consistent style makes call
sites uniform, helps editor tooling, and reduces friction in code review.

Keep an established external API or required interface spelling when renaming would break
compatibility. Suppress those declarations rather than changing the public function selector.

## Example

```solidity
function get_balance() external view returns (uint256);
function GetBalance()  external view returns (uint256);
```

Use instead:

```solidity
function getBalance() external view returns (uint256);
```

## Configuration

Set `mixed_case_exceptions` under `[lint.lint_specific]` in `foundry.toml` to replace the default
list of allowed uppercase patterns (`ERC`, `URI`, `ID`, `URL`, `API`, `JSON`, `XML`, `HTML`, `HTTP`,
and `HTTPS`):

```toml
[lint.lint_specific]
mixed_case_exceptions = ["ERC", "URI", "NFT"]
```
