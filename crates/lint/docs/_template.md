# <Lint name>

**Severity**: `<High | Med | Low | Info | Gas | CodeSize>`
**ID**: `<str_id>`

One-paragraph summary of what this lint detects and why it matters.

## What it does

Explain the user-visible pattern the lint flags, not how the detector implements the check.
Include exclusions only when they help the reader interpret or address a warning.

## Why is this bad?

Explain the impact (security, correctness, gas, readability). For a style or policy choice,
replace this heading with `## Why restrict this?` and explain the tradeoff.

## Example

```solidity
// triggering example
```

Use instead:

```solidity
// non-triggering, recommended example
```

## Configuration

Document any inline-config or `foundry.toml` options that affect this lint. Omit this section when
the lint has no additional configuration.
