# Internal function used once

**Severity**: `Info`
**ID**: `internal-function-used-once`

Flags internal functions referenced exactly once in the whole compilation unit.

## What it does

Reports an ordinary internal function, free functions included, that exactly one expression in the unit references, call or value alike. References are resolved through the type checker, so overload selection, the qualified and `using for` forms and import aliases attribute each reference to the right declaration; references are counted across dependencies too, while only functions declared in the project's own sources report.

Out of scope: functions whose name starts with `_` (the hook convention, OpenZeppelin style), `virtual` functions and overrides (they exist for dynamic dispatch, so inlining them is not an option), functions referenced zero times, which are dead code rather than an inlining candidate, functions bound as user-defined operators through `using {f as +} for T`: operator uses are not name references, so their count would lie, and the binding requires a named function, so inlining is not an option either, and recursive functions: a self-recursive function cannot be inlined into a caller (self-references do not count toward the reference count), and mutually recursive helpers whose single references only form the cycle have no caller to inline into. Aderyn's detector of the same name counts identifier references and does not exempt virtual functions or overrides.

## Why restrict this?

A function with a single caller introduces another declaration for the reader to follow. Inlining
short helpers can make the caller easier to read. A helper can still be useful with one caller
when its name explains a distinct operation or separates complex logic; review that tradeoff
before inlining it.

## Example

```solidity
function price(uint256 amount) internal view returns (uint256) {
    return scaled(amount) * rate;
}

function scaled(uint256 amount) internal pure returns (uint256) {
    return amount * 1e18;
}
```

Use instead:

```solidity
function price(uint256 amount) internal view returns (uint256) {
    return amount * 1e18 * rate;
}
```
