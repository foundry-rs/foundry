# Forge lint documentation

This directory contains one markdown file per registered `forge-lint` rule. Each file is referenced
by the lint's `help` URL (`https://getfoundry.sh/forge/linting/<id>`) and is consumed by the
[Foundry book](https://github.com/foundry-rs/book) to render the lint reference page.

## Adding a new lint

When you add a new lint with `declare_forge_lint!`, you **must** also add a documentation file at
`crates/lint/docs/<str_id>.md`. The presence of the file is enforced by the
`registered_lints_have_docs` unit test in [`crates/lint/src/sol/mod.rs`](../src/sol/mod.rs).

Use [`_template.md`](./_template.md) as a starting point.

## File structure

Each lint doc file should follow this structure:

```markdown
# <human-readable lint name>

**Severity**: `<High | Med | Low | Info | Gas | CodeSize>`
**ID**: `<str_id>`

A one-paragraph description of what this lint detects and why it matters.

## What it does

Explain precisely what the lint flags.

## Why is this bad?

Explain the impact (security, correctness, gas, readability).

## Example

### Bad

```solidity
// triggering example
```

### Good

```solidity
// non-triggering, recommended example
```

## Configuration

Document any inline-config or `foundry.toml` options that affect this lint, if any.
```
