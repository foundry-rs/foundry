# Block number captures across roll

**Severity**: `Med`
**ID**: `block-number-across-roll`

## What it does

Warns when a value derived from `block.number` can be used after `vm.roll`, or when raw
block-number reads occur on both sides of a roll in the same call frame. The diagnostic points
to the original raw read.

## Why is this bad?

The EVM block number is constant during a normal transaction. Compilers can therefore reuse,
move, or defer `block.number` reads across calls. Foundry's `vm.roll` changes that environment
inside a test, so a Solidity local is not a guarantee that the earlier number was captured.
This can happen with optimized solc via IR as well as other compiler optimizations.

Use `vm.getBlockNumber()` for test values that need to observe a particular point in time.
Its call result is materialized. Keep normal compiler optimizations enabled.

## Example

```solidity
vm.roll(100);
uint256 saved = block.number; // Warning: this capture can cross the next roll.
vm.roll(200);
vm.roll(saved);
```

Use instead:

Capture through the getter instead:

```solidity
vm.roll(100);
uint256 saved = vm.getBlockNumber();
vm.roll(200);
vm.roll(saved); // Restores 100.
```

For before/after comparisons, use the getter on both sides of the mutation.

## Scope and controls

The rule runs in `forge lint` and the normal build lint stage, including configured test and
script directories. It uses source analysis and does not depend on the selected optimizer
settings. It does not modify compiler output or automatically insert cheatcodes into source.
Production reads without a recognized roll stay quiet.

The analysis follows scalar local aliases, arithmetic, tuples, internal helper arguments and
returns, inherited helpers, and modifier bodies. It recognizes the constant cheatcode address
and the resolved `roll(uint256)` signature, even when the receiver is not named `vm`.
Unrelated methods named `roll` and changes to the timestamp do not trigger this rule.
External call results, including public and external library calls through `delegatecall`,
are treated as materialized values from a separate call frame.

This is a bounded warning, not a complete execution analysis: it visits up to 16,384 nodes,
retains at most 32 paths at statement boundaries, follows at most eight function frames, and
visits at most two loop iterations. It does not prove relationships between runtime conditions
or analyze recursive/indirect calls, low-level cheatcode calls, assembly, or heap/storage aliases.
Known unsigned and boolean locals prune exhausted loops and constant branches. Internal-call
state effects can be joined conservatively, but differing return values and conditional-expression
values are discarded to avoid combining mutually exclusive outcomes. This can miss captures
returned by branching helpers. Absence of a warning does not establish that every test capture
is safe.

Existing severity filters, `exclude_lints`, and inline suppressions apply. Suppress at the raw
capture when its behavior is intentional:

```solidity
// forge-lint: disable-next-line(block-number-across-roll)
uint256 saved = block.number;
```
