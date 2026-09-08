# Timestamp captures across warp

**Severity**: `Med`
**ID**: `block-timestamp-across-warp`

## What it does

Warns when a value derived from `block.timestamp` can be used after `vm.warp`, or when raw
timestamp reads occur on both sides of a warp in the same call frame. The diagnostic points
to the original raw read.

## Why is this bad?

The EVM timestamp is constant during a normal transaction. Compilers can therefore reuse,
move, or defer `block.timestamp` reads across calls. Foundry's `vm.warp` changes that environment
inside a test, so a Solidity local is not a guarantee that the earlier timestamp was captured.
This can happen with optimized solc via IR as well as other compiler optimizations.

Use `vm.getBlockTimestamp()` for test values that need to observe a particular point in time.
Its call result is materialized. Keep normal compiler optimizations enabled.

## Example

### Bad

```solidity
vm.warp(100);
uint256 saved = block.timestamp; // Warning: this capture can cross the next warp.
vm.warp(200);
vm.warp(saved);
```

### Good

Capture through the getter instead:

```solidity
vm.warp(100);
uint256 saved = vm.getBlockTimestamp();
vm.warp(200);
vm.warp(saved); // Restores 100.
```

For before/after comparisons, use the getter on both sides of the mutation.

## Scope and controls

The rule runs in `forge lint` and the normal build lint stage, including configured test and
script directories. It uses source analysis and does not depend on the selected optimizer
settings. It does not modify compiler output or automatically insert cheatcodes into source.
Production reads without a recognized warp stay quiet.

The analysis follows scalar local aliases, arithmetic, tuples, internal helper arguments and
returns, inherited helpers, and modifier bodies. It recognizes the constant cheatcode address
and the resolved `warp(uint256)` signature, even when the receiver is not named `vm`.
Unrelated methods named `warp` and changes to the block number do not trigger this rule.
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
// forge-lint: disable-next-line(block-timestamp-across-warp)
uint256 saved = block.timestamp;
```

This is separate from `block-timestamp`, which warns about validator-influenced comparisons in
production code. This rule addresses compiler materialization in Foundry tests and scripts.
