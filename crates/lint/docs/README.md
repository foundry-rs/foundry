# Forge lint documentation

This directory contains one markdown file per registered `forge-lint` rule. Each file is referenced
by the lint's `help` URL (`https://getfoundry.sh/forge/linting/<id>`) and is consumed by the
[Foundry book](https://github.com/foundry-rs/book) to render the lint reference page.

## Adding a new lint

When you add a new lint with `declare_forge_lint!`, you **must** also add a documentation file at
`crates/lint/docs/<str_id>.md`. The `registered_lints_have_docs` unit test in
[`crates/lint/src/sol/mod.rs`](../src/sol/mod.rs) enforces the file's presence, registered ID and
severity, required section order, and paired examples.

Use [`_template.md`](./_template.md) as a starting point.

Follow the [lint writing style guide](../../../docs/dev/lintrules.md#lint-writing-style) for naming,
diagnostics, and examples. This documentation format follows
[Clippy's lint documentation guidance](https://doc.rust-lang.org/clippy/development/adding_lints.html#documentation).

- `What it does` describes the conditions actually checked, including relevant exclusions.
- `Why is this bad?` explains the consequence. For style or policy choices that can reasonably be
  allowed, use `Why restrict this?` instead and explain the tradeoff. Choose by purpose, not severity.
- `Example` contains a short triggering Solidity example, then `Use instead:` and a non-triggering
  alternative. Preserve the intended behavior where possible and explain tradeoffs where it changes.
  State any required context, imports, compiler version, or lint configuration. If local suppression
  is the appropriate alternative, explain why the flagged code is acceptable before showing it.
- Use backticks for code in prose. Keep security, correctness, and gas claims specific; explain
  limitations instead of implying every match is a bug or every suggested change is always safe.
- Keep matching reference pages in the [Foundry Book](https://github.com/foundry-rs/book) synchronized.

## File structure

Each lint doc file should follow this structure:

````markdown
# <human-readable lint name>

**Severity**: `<High | Med | Low | Info | Gas | CodeSize>`
**ID**: `<str_id>`

A one-paragraph description of what this lint detects and why it matters.

## What it does

Explain precisely what the lint flags.

## Why is this bad?

Explain the impact (security, correctness, gas, readability).

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
````
