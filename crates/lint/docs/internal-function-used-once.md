# Internal function used once

**Severity**: `Info`
**ID**: `internal-function-used-once`

Flags internal functions referenced exactly once in the whole compilation unit.

## What it does

Reports an ordinary internal function, free functions included, that exactly one expression in the unit references, call or value alike. References are resolved through the type checker, so overload selection, the qualified and `using for` forms and import aliases attribute each reference to the right declaration; references are counted across dependencies too, while only functions declared in the project's own sources report.

Out of scope: functions whose name starts with `_` (the hook convention, OpenZeppelin style), `virtual` functions and overrides (they exist for dynamic dispatch, so inlining them is not an option), and functions referenced zero times, which are dead code rather than an inlining candidate. Aderyn's detector of the same name counts identifier references and does not exempt virtual functions or overrides.

## Why is this bad?

A function with a single caller adds a name, a signature and a jump for the reader without giving the logic a second user. Inlining it usually makes the caller easier to read; if the separation genuinely helps, a second use will justify it eventually.

## Example

### Bad

```solidity
function price(uint256 amount) internal view returns (uint256) {
    return scaled(amount) * rate;
}

function scaled(uint256 amount) internal pure returns (uint256) {
    return amount * 1e18;
}
```

### Good

```solidity
function price(uint256 amount) internal view returns (uint256) {
    return amount * 1e18 * rate;
}
```
