# Forge lint documentation

This directory contains one markdown file per registered `forge-lint` rule. Each file documents
the page targeted by the lint's `help` URL (`https://getfoundry.sh/forge/linting/<id>`).
Publishing that page in the [Foundry book](https://github.com/foundry-rs/book) is a separate step;
adding a file here does not make the public URL available immediately.

## Adding a new lint

When you add a new lint with `declare_forge_lint!`, you **must** also add a documentation file at
`crates/lint/docs/<str_id>.md`. The `registered_lints_have_docs` unit test in
[`crates/lint/src/sol/mod.rs`](../src/sol/mod.rs) enforces the file's presence, registered ID and
severity, and required section order.

Use [`_template.md`](./_template.md) as a starting point.

The Forge CLI's `ensure_lint_rule_docs` test checks the documentation in this checkout and the
canonical help URLs. Regular tests do not depend on the Book's deployment state. After publishing
the pages, audit the live site explicitly with:

```sh
cargo test -p forge --test cli lint::ensure_published_lint_rule_docs -- --exact --ignored
```

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

Document any inline-config or `foundry.toml` options that affect this lint. Omit this section when
the lint has no additional configuration.
```
