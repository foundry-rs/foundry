# Forge lint documentation

This directory contains the canonical Markdown explanation for each registered `forge-lint` rule.
Include documentation changes in the same Foundry PR as the lint. The
[Foundry Book](https://github.com/foundry-rs/book)'s weekly update imports these files with
`import:lints` and generates the reference pages and navigation. The lint's `help` URL points to
the published page at `https://getfoundry.sh/forge/linting/<id>`.

## Adding a new lint

When you add a new lint with `declare_forge_lint!`, you **must** also add a documentation file at
`crates/lint/docs/<str_id>.md`. Run `cargo test -p forge-lint --lib sol::tests` to check
that every registered lint has a page with matching metadata, the expected structure, and a
canonical help URL. These tests also reject pages for unregistered lints and run in normal CI.
The Book's `import:lints` command validates registered metadata and generates navigation;
`import:lints -- --check` validates its committed import.

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
- Write for lint users, not lint implementers. Omit AST/HIR details, alias tracking, traversal
  rules, analysis budgets, diagnostic placement, and comparisons with other detectors. Keep
  caveats when they affect a remedy, explain false positives, or identify important missed cases;
  do not turn a limited check into a claim of complete coverage. Put implementation
  explanations in developer documentation or source comments.
- Start with `What it does`; do not repeat it in an introductory summary. Keep shared severity,
  file-exclusion, and suppression instructions in the linting guide. Omit generic review reminders
  and extra sections that repeat the problem or remedy.

## File structure

The checker requires one title, severity and filename-matching ID, then `What it does`, exactly
one rationale section, and `Example` in that order. Both explanations must contain prose, and
`Example` must contain nonempty `solidity` fences separated by `Use instead:`. Optional
`Configuration`, `Notes`, `Limitations`, and `Known limitations` sections must be nonempty.
It checks structure only, not whether the explanations or Solidity examples are correct.

Each lint doc file should follow this structure:

````markdown
# <human-readable lint name>

**Severity**: `<High | Med | Low | Info | Gas | CodeSize>`
**ID**: `<str_id>`

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
