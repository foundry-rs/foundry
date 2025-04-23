# Cheatcode Mutability Review (`view` / `pure`)

This document tracks the Foundry cheatcodes that are currently marked `external` with no `view` or `pure` mutability modifier.

## Review Criteria

From issue [#10027](https://github.com/foundry-rs/foundry/issues/10027):

- If a cheatcode **modifies observable state for later cheatcode calls** (EVM, interpreter, filesystem), it is neither `view` nor `pure`
- If a cheatcode **depends on prior cheatcode calls or reads test environment state**, it is `view`
- If a cheatcode **has no side effects and doesn’t depend on test state**, it is `pure`

## Checklist

| Cheatcode ID | Function Signature | Proposed Mutability |
|--------------|--------------------|----------------------|
| `createWallet_1` | `function createWallet(uint256)` | `TBD` |
| `sign_0` | `function sign(...)` | `TBD` |
| ... | ... | ... |